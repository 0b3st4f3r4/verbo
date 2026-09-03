//! Lado servidor do protocolo FXP v1.1 — o `fxpd` embutido
//! (docs/FXP-SCHEMA-v1.md §7: "servidor de referência").
//!
//! Máquina de estados **por conexão**: `AUTH? → CAPS → trabalho`
//! (§4.5/§4.6). Este módulo é o **dono canônico** do estado de protocolo do
//! servidor; os loops genéricos `transport::serve_unix`/`serve_tcp` continuam
//! canos sem semântica (usados por testes/benches).
//!
//! Semântica de serviço: cada pedido é executado no [`FxpBus`] do servidor —
//! o mesmo barramento do consumidor local (rota/limites/fallback/honestidade
//! §4.3/§4.7 idênticos; dono único da semântica de I/O). O Caderno do peer
//! registra o que aconteceu do lado dele.
//!
//! Recursos v1.1 (todos negociados — nunca presumidos):
//! - `TIMESTAMP` (§5): respostas de leitura/atuação carimbadas com o
//!   instante físico do servidor (µs desde o epoch UNIX) — anotação de
//!   laboratório; o Caderno segue no relógio virtual.
//! - `BATCH` (§4.7): `READ_BATCH` com resultado por item, erro honesto.
//! - `LZ4` (§4.8): respostas acima do threshold partem comprimidas.

use crate::auth;
use crate::bus::{FxpBus, Route};
use crate::schema::{
    self, caps, flag, op, reason, AckAct, BatchResult, Body, DeviceDesc, Message, WireValue,
    AUTH_SCHEME_PSK_HMAC_SHA256,
};
use crate::transport::{read_frame, Current, Server, TransportError};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use vbl_runtime::fxp::{ActOutcome, Fxp, Limit, Value};
use vbl_runtime::ledger::Ledger;

/// Política e recursos do servidor (§4.5/§4.6). Default: v1.0 puro —
/// nenhuma capacidade anunciada, sem PSK (o wire default é bit a bit v1.0).
#[derive(Debug, Clone, Default)]
pub struct PeerConfig {
    /// PSK presente ⇒ handshake `AUTH_*` obrigatório antes de qualquer outra
    /// mensagem; violação ⇒ fechamento sem razão (fail closed — §4.6).
    pub psk: Option<Vec<u8>>,
    /// Capacidades anunciadas (bits `caps::*`; bits reservados ignorados).
    pub caps: u16,
    /// TLS presente (v1.2 §7) ⇒ o TCP fala TLS 1.3 sob os frames
    /// (confidencialidade/MAC por frame); peer sem TLS falha o handshake
    /// (nunca texto plano). Só se aplica a TCP — Unix local não cifra.
    pub tls: Option<crate::tls::TlsAccept>,
}

/// O servidor FXP: barramento + Caderno do peer + política.
pub struct PeerServer {
    bus: Arc<Mutex<FxpBus>>,
    ledger: Arc<Mutex<dyn Ledger + Send>>,
    config: PeerConfig,
}

impl PeerServer {
    /// Servidor sobre um barramento novo e um Caderno novo.
    pub fn new(bus: FxpBus, ledger: impl Ledger + Send + 'static, config: PeerConfig) -> Self {
        Self {
            bus: Arc::new(Mutex::new(bus)),
            ledger: Arc::new(Mutex::new(ledger)),
            config,
        }
    }

    /// Servidor compartilhando barramento/Caderno já construídos (o `fxpd`
    /// do CLI monta o bus uma vez e serve N conexões).
    pub fn shared(
        bus: Arc<Mutex<FxpBus>>,
        ledger: Arc<Mutex<dyn Ledger + Send>>,
        config: PeerConfig,
    ) -> Self {
        Self {
            bus,
            ledger,
            config,
        }
    }

    /// Config (probe/diagnóstico).
    pub fn config(&self) -> &PeerConfig {
        &self.config
    }
}

/// Servidor Unix com máquina de estados v1.1 por conexão.
pub fn serve_unix_peer(server: &PeerServer, path: &Path) -> Result<Server, TransportError> {
    // TLS é do TCP remoto (§7): unix local não cifra — config equivocada
    // falha na construção (nunca "serve plano ignorando o tls").
    if server.config.tls.is_some() {
        return Err(TransportError::ConnectionFailed(
            "tls configurado em transporte unix — TLS só se aplica a tcp (v1.2 §7)".into(),
        ));
    }
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?;
    let desligar = Arc::new(AtomicBool::new(false));
    let flag_off = desligar.clone();
    let identifier = format!("unix:{}", path.display());
    let path = path.to_path_buf();
    let bus = server.bus.clone();
    let ledger = server.ledger.clone();
    let config = server.config.clone();
    let handle = std::thread::spawn(move || {
        while !flag_off.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((flow, _)) => {
                    let (bus, ledger, config, flag_conn) = (
                        bus.clone(),
                        ledger.clone(),
                        config.clone(),
                        flag_off.clone(),
                    );
                    std::thread::spawn(move || {
                        serve_connection(Current::Unix(flow), &bus, &ledger, &config, &flag_conn)
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
        let _ = std::fs::remove_file(&path);
    });
    Ok(Server::from_parts(identifier, desligar, Some(handle)))
}

/// Servidor TCP (porta efêmera; devolve a porta sorteada) — mesma máquina
/// de estados do Unix. Escuta em **0.0.0.0**: o beacon multicast (§4.9)
/// anuncia o endereço da interface de saída — com o servidor em todas as
/// interfaces, o IP anunciado é sempre conectável.
pub fn serve_tcp_peer(server: &PeerServer) -> Result<(Server, u16), TransportError> {
    serve_tcp_peer_port(server, 0)
}

/// Variante com porta explícita (`--serve tcp:PORTA`; 0 = efêmera).
pub fn serve_tcp_peer_port(
    server: &PeerServer,
    port: u16,
) -> Result<(Server, u16), TransportError> {
    // TLS v1.2 (§7): config validada UMA vez no arranque — PEM ruim ⇒ erro
    // honesto antes do 1º cliente; o Arc compartilhado serve N conexões.
    let tls_cfg: Option<Arc<rustls::ServerConfig>> = match &server.config.tls {
        Some(a) => {
            Some(Arc::new(crate::tls::server_config(a).map_err(|e| {
                TransportError::ConnectionFailed(e.to_string())
            })?))
        }
        None => None,
    };
    let listener = TcpListener::bind(("0.0.0.0", port))
        .map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?;
    let desligar = Arc::new(AtomicBool::new(false));
    let flag_off = desligar.clone();
    let esquema = if tls_cfg.is_some() { "tcps" } else { "tcp" };
    let identifier = format!("{esquema}:127.0.0.1:{port}");
    let bus = server.bus.clone();
    let ledger = server.ledger.clone();
    let config = server.config.clone();
    let handle = std::thread::spawn(move || {
        while !flag_off.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (bus, ledger, config, flag_conn) = (
                        bus.clone(),
                        ledger.clone(),
                        config.clone(),
                        flag_off.clone(),
                    );
                    let tls_conn = tls_cfg.clone();
                    std::thread::spawn(move || {
                        // Handshake TLS antes da máquina de estados: quem fala
                        // texto plano contra servidor TLS morre aqui (§7).
                        let flow = match tls_conn {
                            Some(cfg) => match crate::tls::server_stream(cfg, stream) {
                                Ok(f) => Current::TlsServer(f),
                                Err(_) => return, // handshake falhou: fecha, segue
                            },
                            None => Current::Tcp(stream),
                        };
                        serve_connection(flow, &bus, &ledger, &config, &flag_conn)
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
    });
    Ok((Server::from_parts(identifier, desligar, Some(handle)), port))
}

/// Instante físico do servidor (µs desde o epoch UNIX) — §5.
fn now_unix_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Máquina de estados de UMA conexão (thread própria): AUTH? → CAPS → trabalho.
fn serve_connection(
    mut flow: Current,
    bus: &Arc<Mutex<FxpBus>>,
    ledger: &Arc<Mutex<dyn Ledger + Send>>,
    config: &PeerConfig,
    shutdown: &AtomicBool,
) {
    flow.set_timeout(Duration::from_millis(250));
    let mut rest: Vec<u8> = Vec::new();

    // v1.3 §7: o CAPS do cliente pode ter chegado como 0-RTT (early data do
    // TLS, na conexão retomada) — drena ANTES do laço; é a primeira entrada
    // da máquina de estados. Sem early data, vazio e nada muda.
    if let Some(early) = flow.take_early_data() {
        rest = early;
    }

    // ---- AUTH (§4.6): com PSK, o servidor FALA PRIMEIRO (challenge). -------
    let server_nonce = if config.psk.is_some() {
        match auth::nonce() {
            Ok(n) => n,
            Err(_) => return, // sem RNG não há handshake honesto
        }
    } else {
        [0u8; auth::NONCE_LEN]
    };
    if config.psk.is_some()
        && write_frame(
            &mut flow,
            &Message::auth_challenge(AUTH_SCHEME_PSK_HMAC_SHA256, server_nonce, 0),
            0,
            None,
        )
        .is_err()
    {
        return;
    }
    let mut authenticated = config.psk.is_none();

    let mut caps_negociadas: u16 = 0;
    // v1.2 §4.8: dict derivado do registro LOCAL (o servidor publica os
    // nomes que viram dicionário). Só é usado depois do HELLO do cliente —
    // prova de que o outro lado já derivou os mesmos bytes.
    let mut dict_local: Option<schema::compress::DictConexao> = None;
    let mut dict_ready = false;
    // v1.4 §4.8: o dicionário TREINADO fica reservado quando `ZSTD_V` é
    // concedido — só vira `DictConexao::ZstdV` (id 4) após o `DICT_SYNC`
    // com hash casado; até lá as respostas saem no id 2/LZ4/plano.
    let mut dict_treinado: Option<Vec<u8>> = None;
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        match read_frame(
            &mut flow,
            &mut rest,
            dict_local.as_ref().filter(|_| dict_ready),
        ) {
            Ok(Some(msg)) => {
                // Fail closed: com PSK, só AUTH_RESPONSE é aceita pré-auth
                // (§4.6 — qualquer outra opcode ⇒ fechamento sem razão).
                if !authenticated {
                    authenticated = match handle_auth(&msg, config, &server_nonce, &mut flow) {
                        AuthResult::Ok => true,
                        AuthResult::Close => return,
                    };
                    continue;
                }
                if msg.opcode == op::BYE {
                    return;
                }
                let eh_hello = msg.opcode == op::HELLO;
                if let Some(resp) = dispatch(
                    &msg,
                    bus,
                    ledger,
                    config,
                    &mut caps_negociadas,
                    &mut dict_local,
                    &mut dict_treinado,
                ) {
                    // O HELLO de resposta nunca sai com dict (o cliente só
                    // terá o dicionário depois de recebê-lo).
                    let dict = if dict_ready && !eh_hello {
                        dict_local.as_ref()
                    } else {
                        None
                    };
                    if write_frame(&mut flow, &resp, caps_negociadas, dict).is_err() {
                        return;
                    }
                    if eh_hello && dict_local.is_some() {
                        dict_ready = true;
                    }
                }
            }
            Ok(None) => continue, // frame incompleto; lê mais
            Err(TransportError::Timeout) => continue,
            Err(_) => return,
        }
    }
}

enum AuthResult {
    Ok,
    Close,
}

/// Passo de autenticação (§4.6): exige `AUTH_RESPONSE`; chave errada ou
/// nonce reutilizado ⇒ fechamento limpo **sem** `AUTH_OK`.
fn handle_auth(
    msg: &Message,
    config: &PeerConfig,
    server_nonce: &[u8; auth::NONCE_LEN],
    flow: &mut Current,
) -> AuthResult {
    let (Some(key), Body::AuthResponse { nonce, mac }) = (config.psk.as_deref(), &msg.body) else {
        return AuthResult::Close; // não-autenticado falando outra coisa
    };
    if auth::verify(key, nonce, server_nonce, mac) {
        let _ = write_frame(flow, &Message::auth_ok(msg.seq), 0, None);
        AuthResult::Ok
    } else {
        AuthResult::Close
    }
}

/// Dispatch pós-auth. `caps_negociadas` é estado **desta conexão** — recurso
/// sem `CAPS_OK` não executa (§4.5).
fn dispatch(
    msg: &Message,
    bus: &Arc<Mutex<FxpBus>>,
    ledger: &Arc<Mutex<dyn Ledger + Send>>,
    config: &PeerConfig,
    caps_negociadas: &mut u16,
    dict_local: &mut Option<schema::compress::DictConexao>,
    dict_treinado: &mut Option<Vec<u8>>,
) -> Option<Message> {
    match msg.opcode {
        op::CAPS => {
            let Body::Caps { capabilities } = msg.body else {
                return None;
            };
            // Interseção pedidos × anunciados; bits reservados ignorados
            // (peers antigos ignoram bits novos no decode ⇒ interseção sem
            // eles — a promoção v1.2/v1.3/v1.4 é segura por construção).
            *caps_negociadas = capabilities & config.caps & !caps::RESERVED;
            let nomes: Vec<String> = bus
                .lock()
                .map(|b| {
                    b.registry_rico()
                        .devices()
                        .map(|d| d.name.clone())
                        .collect()
                })
                .unwrap_or_default();
            // v1.4 §4.8: ZSTD_V (id 4) concedido SÓ com DICT também
            // concedido e SÓ quando o dicionário TREINA. O treinado fica
            // RESERVADO até o `DICT_SYNC` (hash casado) — até lá o servidor
            // fala id 2/LZ4/plano. Treino impossível ⇒ ZSTD_V e ZSTD saem
            // da interseção (o id 3 sem verificação não combina com cliente
            // que pediu verificação).
            if *caps_negociadas & caps::ZSTD_V != 0 {
                match (*caps_negociadas & caps::DICT != 0)
                    .then(|| schema::compress::zstd_dict_from_registry(&nomes))
                    .flatten()
                {
                    Some(treinado) => {
                        *dict_treinado = Some(treinado);
                        *dict_local = Some(schema::compress::DictConexao::Lz4(
                            schema::compress::dict_from_registry(&nomes),
                        ));
                    }
                    None => *caps_negociadas &= !(caps::ZSTD_V | caps::ZSTD),
                }
            } else if *caps_negociadas & caps::ZSTD != 0 {
                // v1.3 §4.8: ZSTD concedido SÓ com DICT também concedido (o
                // gatilho do HELLO é o mesmo) e SÓ quando o dicionário
                // TREINA — sem treino, degradação honesta: o bit sai da
                // interseção.
                match (*caps_negociadas & caps::DICT != 0)
                    .then(|| schema::compress::zstd_dict_from_registry(&nomes))
                    .flatten()
                {
                    Some(d) => *dict_local = Some(schema::compress::DictConexao::Zstd(d)),
                    None => *caps_negociadas &= !caps::ZSTD,
                }
            }
            // v1.2 §4.8: DICT concedido ⇒ deriva o dicionário do registro
            // servido (os mesmos bytes que o cliente obterá via HELLO).
            if *caps_negociadas & caps::DICT != 0 && dict_local.is_none() {
                *dict_local = Some(schema::compress::DictConexao::Lz4(
                    schema::compress::dict_from_registry(&nomes),
                ));
            }
            Some(Message::caps_ok(*caps_negociadas, msg.seq))
        }
        op::DICT_SYNC => {
            // v1.4 §4.8: hash casado ⇒ id 4 liberado nos DOIS sentidos
            // (respostas partem com id 4 e frames id 4 do cliente decodificam);
            // divergente ⇒ resposta honesta com o par do servidor — o cliente
            // degrada para o id 2 SEM tentar frame que falharia.
            let Body::DictSync { dict_hash, .. } = &msg.body else {
                return None;
            };
            let Some(treinado) = dict_treinado.as_ref() else {
                return None; // sem ZSTD_V concedido, DICT_SYNC é violação
            };
            let meu_hash = schema::compress::hash_dict(treinado);
            if meu_hash == *dict_hash {
                *dict_local = Some(schema::compress::DictConexao::ZstdV(treinado.clone()));
            }
            Some(Message::dict_sync_ok(
                schema::compress::zstd_version(),
                meu_hash,
                msg.seq,
            ))
        }
        op::READ => {
            let mut resp = handle_read(bus, ledger, &msg.name, msg.seq);
            if (*caps_negociadas) & caps::TIMESTAMP != 0 {
                resp = resp.with_timestamp(now_unix_us());
            }
            Some(resp)
        }
        op::READ_BATCH => {
            if (*caps_negociadas) & caps::BATCH == 0 {
                return None; // violação de protocolo: fechar sem responder
            }
            let Body::ReadBatch { names } = &msg.body else {
                return None;
            };
            // §4.7: item sintético no lote ⇒ frame inteiro marcado (marca
            // conservadora — nunca deixa valor simulado passar sem marca).
            let any_synthetic = bus
                .lock()
                .map(|b| {
                    names
                        .iter()
                        .any(|n| matches!(b.route_of(n), Some(Route::Simulator)))
                })
                .unwrap_or(false);
            let mut resp = handle_batch(bus, ledger, names, msg.seq);
            if any_synthetic {
                resp.flags |= flag::SYNTHETIC;
            }
            if (*caps_negociadas) & caps::TIMESTAMP != 0 {
                resp = resp.with_timestamp(now_unix_us());
            }
            Some(resp)
        }
        op::ACT => {
            let value = match &msg.body {
                Body::Act { value } => value,
                _ => return None,
            };
            let mut resp = handle_act(bus, ledger, &msg.name, value, msg.seq);
            if (*caps_negociadas) & caps::TIMESTAMP != 0 {
                resp = resp.with_timestamp(now_unix_us());
            }
            Some(resp)
        }
        op::HEARTBEAT => Some(Message::heartbeat_ack(true, msg.seq)),
        op::HELLO => Some(Message::hello(hello_do_registro(bus), msg.seq)),
        _ => None, // BYE tratado antes; opcodes de auth pós-handshake ⇒ ignora
    }
}

/// Escreve a resposta com compressão negociada (§4.8/v1.2/v1.3) — o codec
/// decide "só quando compensa"; com dict pronto, o id 3 (zstd treinado)
/// ou o id 2 (LZ4) têm precedência sobre o LZ4 simples (mesma regra do
/// cliente em `transport::Connection`).
fn write_frame(
    flow: &mut Current,
    msg: &Message,
    caps_negociadas: u16,
    dict: Option<&schema::compress::DictConexao>,
) -> Result<(), TransportError> {
    let frame = match dict {
        // v1.4 §4.8: dicionário treinado VERIFICADO (id 4) tem precedência —
        // só existe aqui depois do DICT_SYNC com hash casado.
        Some(schema::compress::DictConexao::ZstdV(dict)) => {
            let mut f = Vec::with_capacity(schema::HEADER_LEN + msg.name.len() + 64);
            schema::encode_with_zstd_dict_v(msg, dict, &mut f).map_err(TransportError::from)?;
            f
        }
        Some(schema::compress::DictConexao::Zstd(dict)) => {
            let mut f = Vec::with_capacity(schema::HEADER_LEN + msg.name.len() + 64);
            schema::encode_with_zstd_dict(msg, dict, &mut f).map_err(TransportError::from)?;
            f
        }
        Some(schema::compress::DictConexao::Lz4(dict)) => {
            let mut f = Vec::with_capacity(schema::HEADER_LEN + msg.name.len() + 64);
            schema::encode_with_compression_dict(msg, dict, &mut f)
                .map_err(TransportError::from)?;
            f
        }
        None if caps_negociadas & caps::LZ4 != 0 => {
            let mut f = Vec::with_capacity(schema::HEADER_LEN + msg.name.len() + 64);
            schema::encode_with_compression(msg, &mut f).map_err(TransportError::from)?;
            f
        }
        None => schema::encode_to_vec(msg).map_err(TransportError::from)?,
    };
    flow.write_all(&frame)
        .map_err(|e| TransportError::Broken(format!("escrita: {e}")))
}

/// `READ` individual: roteia pelo barramento do servidor (honestidade §4.7
/// do lado de cá: falha vira `READ_ERR`, nunca valor fabricado).
fn handle_read(
    bus: &Arc<Mutex<FxpBus>>,
    ledger: &Arc<Mutex<dyn Ledger + Send>>,
    name: &str,
    seq: u32,
) -> Message {
    let mut bus = match bus.lock() {
        Ok(b) => b,
        Err(_) => return Message::read_err(reason::BUSY, seq),
    };
    let mut led = match ledger.lock() {
        Ok(l) => l,
        Err(_) => return Message::read_err(reason::BUSY, seq),
    };
    let synthetic = matches!(bus.route_of(name), Some(Route::Simulator));
    match bus.read_sensor(name, &mut *led) {
        Ok(v) => {
            let canonical = bus.registry_rico().canonical_of(name).to_string();
            Message::read_ok(v, &canonical, synthetic, seq)
        }
        Err(_) => Message::read_err(reason::INACCESSIBLE, seq),
    }
}

/// `READ_BATCH` (§4.7): resultado por item — erro honesto, nunca 0.0.
fn handle_batch(
    bus: &Arc<Mutex<FxpBus>>,
    ledger: &Arc<Mutex<dyn Ledger + Send>>,
    names: &[String],
    seq: u32,
) -> Message {
    let mut bus = match bus.lock() {
        Ok(b) => b,
        Err(_) => {
            return Message::read_batch_ok(
                vec![BatchResult::Err {
                    reason: reason::BUSY,
                }],
                seq,
            )
        }
    };
    let mut led = match ledger.lock() {
        Ok(l) => l,
        Err(_) => {
            return Message::read_batch_ok(
                vec![BatchResult::Err {
                    reason: reason::BUSY,
                }],
                seq,
            )
        }
    };
    let results = names
        .iter()
        .map(|name| {
            let canonical = bus.registry_rico().canonical_of(name).to_string();
            if !bus.registry_rico().contains(&canonical) {
                return BatchResult::Err {
                    reason: reason::NOT_REGISTERED,
                };
            }
            match bus.read_sensor(&canonical, &mut *led) {
                Ok(v) => BatchResult::Ok {
                    value: v,
                    canonical,
                },
                Err(_) => BatchResult::Err {
                    reason: reason::INACCESSIBLE,
                },
            }
        })
        .collect();
    Message::read_batch_ok(results, seq)
}

/// `ACT` (§4.3): validação/limites/fallback são do barramento; o resultado
/// vira `ACT_ACK` com o status espelhado.
fn handle_act(
    bus: &Arc<Mutex<FxpBus>>,
    ledger: &Arc<Mutex<dyn Ledger + Send>>,
    name: &str,
    value: &WireValue,
    seq: u32,
) -> Message {
    // Conversão WireValue → Value do runtime (preserva Str × Ident).
    let value = match value {
        WireValue::Num(n) => Value::Num(*n),
        WireValue::Str(s) => Value::Str(s.clone()),
        WireValue::Ident(s) => Value::Ident(s.clone()),
    };
    let mut bus = match bus.lock() {
        Ok(b) => b,
        Err(_) => return Message::act_ack(AckAct::Unavailable, false, seq),
    };
    let mut led = match ledger.lock() {
        Ok(l) => l,
        Err(_) => return Message::act_ack(AckAct::Unavailable, false, seq),
    };
    let outcome = bus.act(name, value, &mut *led);
    let (status, fallback) = ack_de_outcome(outcome);
    Message::act_ack(status, fallback, seq)
}

/// Espelho `ActOutcome` → `AckAct` (inverso do mapa do `bus.deliver_remote`).
fn ack_de_outcome(outcome: ActOutcome) -> (AckAct, bool) {
    match outcome {
        ActOutcome::Delivered => (AckAct::Delivered, false),
        ActOutcome::Rejected { limit, limit_value } => {
            let limit = match limit {
                Limit::Min => 0,
                Limit::Max => 1,
                Limit::SafetyLimit => 2,
            };
            (AckAct::Rejected { limit, limit_value }, false)
        }
        ActOutcome::MissingActor => (AckAct::MissingActor, false),
        ActOutcome::Unavailable => (AckAct::Unavailable, false),
        ActOutcome::InvalidValue { reason } => (AckAct::InvalidValue { reason }, false),
        ActOutcome::FallbackExecuted { alternativo } => {
            (AckAct::FallbackExecuted { alternativo }, true)
        }
        ActOutcome::FallbackExhausted => (AckAct::FallbackExhausted, false),
    }
}

/// `HELLO` de resposta: o registro DO servidor (§4.4).
fn hello_do_registro(bus: &Arc<Mutex<FxpBus>>) -> Vec<DeviceDesc> {
    let bus = match bus.lock() {
        Ok(b) => b,
        Err(_) => return vec![],
    };
    bus.registry_rico()
        .devices()
        .map(|d| d.to_device_desc())
        .collect()
}
