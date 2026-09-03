//! Transporte do schema v1 sobre stream: Unix (local entre processos) e TCP
//! (remoto) — docs/FXP-SCHEMA-v1.md §7. Ack correlacionado por `seq` com
//! timeout de parede (§6); [`serve_unix`]/[`serve_tcp`] são o servidor de
//! referência (testes de integração e `fxpd` embutido).

use crate::schema::{self, Body, Message};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Falha de transporte (distingue timeout de schema — §4.1/§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    ConnectionFailed(String),
    Broken(String),
    /// Ack não chegou no prazo — vira falha de I/O no bus (§4.7), nunca dado.
    Timeout,
    Schema(schema::SchemaError),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::ConnectionFailed(m) => write!(f, "conexão falhou: {m}"),
            TransportError::Broken(m) => write!(f, "conexão quebrada: {m}"),
            TransportError::Timeout => write!(f, "ack não chegou no prazo"),
            TransportError::Schema(e) => write!(f, "violação do schema v1: {e}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<schema::SchemaError> for TransportError {
    fn from(e: schema::SchemaError) -> Self {
        TransportError::Schema(e)
    }
}

impl From<crate::tls::TlsError> for TransportError {
    fn from(e: crate::tls::TlsError) -> Self {
        // Toda falha TLS é de conexão/autenticação do canal — nunca degrada
        // para texto plano (§7 v1.2: fail closed).
        TransportError::ConnectionFailed(e.to_string())
    }
}

/// Fluxo bidirecional: Unix local, TCP remoto ou TCP+TLS remoto (v1.2 §7) —
/// mesma semântica de frame por cima (o TLS é um cano a mais, nunca muda o
/// schema).
#[derive(Debug)]
pub(crate) enum Current {
    Unix(UnixStream),
    Tcp(TcpStream),
    /// Cliente TLS (`tcps:`, pin por impressão digital).
    TlsClient(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
    /// Servidor TLS (`fxpd --tls-cert/--tls-key`).
    TlsServer(rustls::StreamOwned<rustls::ServerConnection, TcpStream>),
}

impl Current {
    pub(crate) fn set_timeout(&self, d: Duration) {
        match self {
            Current::Unix(s) => {
                let _ = s.set_read_timeout(Some(d));
                let _ = s.set_write_timeout(Some(d));
            }
            Current::Tcp(s) => {
                let _ = s.set_read_timeout(Some(d));
                let _ = s.set_write_timeout(Some(d));
            }
            // O handshake paga o orçamento próprio (tls::HANDSHAKE_TIMEOUT);
            // após o aperto de mãos vale o timeout de trabalho do chamador.
            Current::TlsClient(s) => {
                let _ = s.sock.set_read_timeout(Some(d));
                let _ = s.sock.set_write_timeout(Some(d));
            }
            Current::TlsServer(s) => {
                let _ = s.sock.set_read_timeout(Some(d));
                let _ = s.sock.set_write_timeout(Some(d));
            }
        }
    }

    /// v1.3 §7 — 0-RTT: drena os bytes adiantados pelo cliente (o frame
    /// `CAPS` enviado antes do handshake completo). `None` fora de servidor
    /// TLS ou sem early data. Erro de leitura do buffer interno ⇒ vazio
    /// honesto (a máquina de estados segue pelo caminho normal).
    pub(crate) fn take_early_data(&mut self) -> Option<Vec<u8>> {
        match self {
            Current::TlsServer(s) => {
                let mut buf = Vec::new();
                if let Some(mut early) = s.conn.early_data() {
                    let _ = early.read_to_end(&mut buf);
                }
                Some(buf)
            }
            _ => None,
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Current::Unix(s) => s.read(buf),
            Current::Tcp(s) => s.read(buf),
            Current::TlsClient(s) => s.read(buf),
            Current::TlsServer(s) => s.read(buf),
        }
    }

    pub(crate) fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            Current::Unix(s) => s.write_all(buf),
            Current::Tcp(s) => s.write_all(buf),
            Current::TlsClient(s) => s.write_all(buf),
            Current::TlsServer(s) => s.write_all(buf),
        }
    }
}

/// Conexão cliente falando frames v1.1/v1.2 (docs/FXP-SCHEMA-v1.md §2). O
/// estado de recursos negociados (`CAPS`, §4.5) e do dicionário compartilhado
/// (v1.2 §4.8) vive aqui: nenhum frame com recurso novo parte sem
/// `negotiate()` confirmado (e, para dict, sem o `HELLO` completo) — o
/// cliente falha fechado.
#[derive(Debug)]
pub struct Connection {
    current: Current,
    /// Interseção pedidos × concedidos do handshake `CAPS` (0 = nada).
    negotiated_caps: u16,
    /// Contador próprio de seq dos frames de negociação (não colide com o
    /// espaço de seq do bus — a correlação é por conexão).
    neg_seq: u32,
    /// Dicionário derivado do registro do PEER (v1.2/v1.3 §4.8) — instalado
    /// pelo bus após o `HELLO`; frames só comprimem com dict quando pronto.
    /// O tipo carrega o ALGORITMO (id 2 LZ4 concatenado / id 3 zstd treinado).
    dict: Option<schema::compress::DictConexao>,
    dict_ready: bool,
    /// v1.3 §7: o frame `CAPS` já partiu como 0-RTT durante o handshake TLS
    /// e foi ACEITO — `negotiate` só espera o `CAPS_OK` (não reenvia).
    caps_adiantado: bool,
    /// v1.3 §7: observação imutável da conexão — o frame adiantado partiu e
    /// foi processado (`Some` só em TLS cliente; `None` em Unix/TCP plano).
    tls_0rtt: Option<bool>,
}

impl Connection {
    pub fn unix(path: &Path, timeout: Duration) -> Result<Self, TransportError> {
        let s = UnixStream::connect(path)
            .map_err(|e| TransportError::ConnectionFailed(format!("{}: {e}", path.display())))?;
        s.set_nonblocking(false).ok();
        let c = Connection {
            current: Current::Unix(s),
            negotiated_caps: 0,
            neg_seq: 0,
            dict: None,
            dict_ready: false,
            caps_adiantado: false,
            tls_0rtt: None,
        };
        c.current.set_timeout(timeout);
        Ok(c)
    }

    pub fn tcp(host: &str, port: u16, timeout: Duration) -> Result<Self, TransportError> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| TransportError::ConnectionFailed(format!("{host}:{port}: {e}")))?
            .next()
            .ok_or_else(|| {
                TransportError::ConnectionFailed(format!("{host}:{port} sem endereço"))
            })?;
        let s = TcpStream::connect(addr)
            .map_err(|e| TransportError::ConnectionFailed(format!("{addr}: {e}")))?;
        let c = Connection {
            current: Current::Tcp(s),
            negotiated_caps: 0,
            neg_seq: 0,
            dict: None,
            dict_ready: false,
            caps_adiantado: false,
            tls_0rtt: None,
        };
        c.current.set_timeout(timeout);
        Ok(c)
    }

    /// Conexão TLS (`tcps:`, v1.2 §7): TCP + rustls TLS 1.3 sob os frames —
    /// confidencialidade e MAC por frame. A confiança é a impressão digital
    /// (pin fixo v1.2 ou TOFU v1.3): divergência/recusa ⇒ falha fechada,
    /// **nunca** texto plano (§4.6: falha de segurança é terminativa).
    ///
    /// v1.3 §7: com `early_caps = Some(w)` e uma sessão retomável, o frame
    /// `CAPS` parte como **0-RTT** durante o handshake (poupa 1 RTT por
    /// conexão); sem retomada, `negotiate` segue o caminho normal. Com PSK
    /// de aplicação o chamador NÃO adianta CAPS (o servidor fala primeiro
    /// no AUTH §4.6).
    pub fn tcp_tls(
        host: &str,
        port: u16,
        confianca: &crate::tls::ConfiancaCliente,
        timeout: Duration,
        early_caps: Option<u16>,
    ) -> Result<Self, TransportError> {
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| TransportError::ConnectionFailed(format!("{host}:{port}: {e}")))?
            .next()
            .ok_or_else(|| {
                TransportError::ConnectionFailed(format!("{host}:{port} sem endereço"))
            })?;
        let s = TcpStream::connect(addr)
            .map_err(|e| TransportError::ConnectionFailed(format!("{addr}: {e}")))?;
        let name = rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|e| TransportError::ConnectionFailed(format!("{host}: nome TLS: {e}")))?;
        let cfg = crate::tls::client_config_cached(confianca)?;
        // O frame adiantado é o próprio CAPS (seq 1 — o mesmo que negotiate
        // usaria); encode sem dicionário/compressão (handshake §4.5).
        let early_frame = early_caps
            .filter(|&w| w != 0)
            .map(|w| schema::encode_to_vec(&Message::caps(w, 1)))
            .transpose()?;
        let (stream, caps_0rtt_aceito) =
            crate::tls::client_stream(cfg, s, name, early_frame.as_deref())?;
        let c = Connection {
            current: Current::TlsClient(stream),
            negotiated_caps: 0,
            neg_seq: 0,
            dict: None,
            dict_ready: false,
            caps_adiantado: caps_0rtt_aceito,
            tls_0rtt: Some(caps_0rtt_aceito),
        };
        c.current.set_timeout(timeout);
        Ok(c)
    }

    /// Envia a mensagem (frame completo). Com `CAPS` bit 0 negociado (§4.5),
    /// frames acima do threshold (§4.8) partem comprimidos em LZ4.
    pub fn enviar(&mut self, msg: &Message) -> Result<(), TransportError> {
        let frame = self.encode_frame(msg)?;
        self.current
            .write_all(&frame)
            .map_err(|e| TransportError::Broken(format!("escrita: {e}")))
    }

    /// Encode do frame conforme as capacidades negociadas (§4.5/§4.8/v1.2):
    /// dict (id 2) tem precedência quando negociado e pronto; LZ4 simples é
    /// o caminho v1.1; sem recursos, o fio plano.
    fn encode_frame(&self, msg: &Message) -> Result<Vec<u8>, TransportError> {
        if self.negotiated_caps & schema::caps::DICT != 0 && self.dict_ready {
            match &self.dict {
                // v1.3 §4.8: zstd treinado (id 3) tem precedência quando
                // negociado — razão maior com os mesmos bytes de gatilho.
                Some(schema::compress::DictConexao::Zstd(dict))
                    if self.negotiated_caps & schema::caps::ZSTD != 0 =>
                {
                    let mut f = Vec::with_capacity(schema::HEADER_LEN + msg.name.len() + 64);
                    schema::encode_with_zstd_dict(msg, dict, &mut f)?;
                    return Ok(f);
                }
                Some(schema::compress::DictConexao::Lz4(dict)) => {
                    let mut f = Vec::with_capacity(schema::HEADER_LEN + msg.name.len() + 64);
                    schema::encode_with_compression_dict(msg, dict, &mut f)?;
                    return Ok(f);
                }
                _ => {}
            }
        }
        if self.negotiated_caps & schema::caps::LZ4 != 0 {
            let mut f = Vec::with_capacity(schema::HEADER_LEN + msg.name.len() + 64);
            schema::encode_with_compression(msg, &mut f)?;
            return Ok(f);
        }
        Ok(schema::encode_to_vec(msg)?)
    }

    /// Recebe **um** frame respeitando o prazo total (`timeout` de parede).
    pub fn receive(&mut self, timeout: Duration) -> Result<Message, TransportError> {
        let deadline = Instant::now() + timeout;
        let mut read = |buf: &mut [u8]| -> Result<usize, TransportError> {
            loop {
                // Prazo recalculado a cada iteração: cada `read` bloqueia no
                // máximo o que falta; EAGAIN no fim do prazo → Timeout.
                let remains = deadline.saturating_duration_since(Instant::now());
                if remains.is_zero() {
                    return Err(TransportError::Timeout);
                }
                self.current.set_timeout(remains);
                match self.current.read(buf) {
                    Ok(0) => return Err(TransportError::Broken("fim do fluxo".into())),
                    Ok(n) => return Ok(n),
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::Interrupted =>
                    {
                        continue;
                    }
                    Err(e) => return Err(TransportError::Broken(format!("leitura: {e}"))),
                }
            }
        };

        let mut prefix = [0u8; 4];
        let mut n_read = 0;
        while n_read < 4 {
            n_read += read(&mut prefix[n_read..])?;
        }
        let total = schema::peek_frame_len(&prefix)?;
        let mut frame = vec![0u8; total];
        frame[..4].copy_from_slice(&prefix);
        let mut n_read = 4;
        while n_read < total {
            n_read += read(&mut frame[n_read..])?;
        }
        let (msg, _) = schema::decode_with_conexao(&frame, self.dict.as_ref())?;
        Ok(msg)
    }

    /// Pedido-resposta: envia e espera o ack com o **mesmo seq** (§5).
    /// Resposta com seq divergente ⇒ conexão dessincronizada (erro, nunca
    /// ack trocado entre comandos).
    pub fn request(&mut self, msg: &Message, timeout: Duration) -> Result<Message, TransportError> {
        self.enviar(msg)?;
        let resp = self.receive(timeout)?;
        if resp.seq != msg.seq {
            return Err(TransportError::Broken(format!(
                "seq dessincronizado: enviado {}, recebido {}",
                msg.seq, resp.seq
            )));
        }
        Ok(resp)
    }

    // -----------------------------------------------------------------
    // v1.1 — Negociação de capacidades (docs/FXP-SCHEMA-v1.md §4.5)
    // -----------------------------------------------------------------

    /// Handshake `CAPS` → `CAPS_OK`: pede `wanted` e guarda a **interseção**
    /// concedida pelo peer. `wanted = 0` é no-op no fio (zera o estado local).
    /// Resposta que não seja `CAPS_OK` ⇒ conexão dessincronizada (erro).
    /// v1.3 §7: com o CAPS já adiantado como 0-RTT, só espera o `CAPS_OK`.
    pub fn negotiate(&mut self, wanted: u16, timeout: Duration) -> Result<u16, TransportError> {
        if wanted == 0 {
            self.negotiated_caps = 0;
            return Ok(0);
        }
        if std::mem::take(&mut self.caps_adiantado) {
            self.neg_seq = 1; // o frame adiantado saiu com seq 1
            let resp = self.receive(timeout)?;
            if resp.seq != self.neg_seq {
                return Err(TransportError::Broken(format!(
                    "seq dessincronizado: enviado {}, recebido {}",
                    self.neg_seq, resp.seq
                )));
            }
            let Body::Caps { capabilities } = resp.body else {
                return Err(TransportError::Broken(
                    "resposta à negociação não é CAPS_OK (§4.5)".into(),
                ));
            };
            self.negotiated_caps = capabilities;
            return Ok(capabilities);
        }
        self.neg_seq = self.neg_seq.wrapping_add(1);
        let req = Message::caps(wanted, self.neg_seq);
        let resp = self.request(&req, timeout)?;
        let Body::Caps { capabilities } = resp.body else {
            return Err(TransportError::Broken(
                "resposta à negociação não é CAPS_OK (§4.5)".into(),
            ));
        };
        self.negotiated_caps = capabilities;
        Ok(capabilities)
    }

    /// Capacidades concedidas pelo peer (0 antes do handshake).
    pub fn negotiated_caps(&self) -> u16 {
        self.negotiated_caps
    }

    /// Tipo do handshake TLS desta conexão (v1.3 §7 — observação para
    /// testes/probe): `Resumed` na retomada de sessão; `None` fora de TLS.
    pub fn tls_handshake_kind(&self) -> Option<rustls::HandshakeKind> {
        match &self.current {
            Current::TlsClient(s) => s.conn.handshake_kind(),
            Current::TlsServer(s) => s.conn.handshake_kind(),
            _ => None,
        }
    }

    /// 0-RTT aceito pelo servidor nesta conexão TLS (v1.3 §7): `Some(true)`
    /// quando um frame adiantado partiu e foi processado; `Some(false)` sem
    /// early data; `None` fora de TLS.
    pub fn tls_0rtt_aceito(&self) -> Option<bool> {
        self.tls_0rtt
    }

    // -----------------------------------------------------------------
    // v1.2 — Dicionário de compressão compartilhado (§4.8)
    // -----------------------------------------------------------------

    /// Instala o dicionário derivado do registro do PEER (id 2, v1.2) e
    /// marca pronto — chamado pelo bus imediatamente após o
    /// [`Self::exchange_hello`].
    pub fn set_dict(&mut self, dict: Vec<u8>) {
        self.dict = Some(schema::compress::DictConexao::Lz4(dict));
        self.dict_ready = true;
    }

    /// Instala o dicionário TREINADO (id 3, v1.3 §4.8) e marca pronto —
    /// exige `caps::ZSTD` negociado; frames acima do threshold partem com o
    /// algoritmo 3.
    pub fn set_zstd_dict(&mut self, dict: Vec<u8>) {
        self.dict = Some(schema::compress::DictConexao::Zstd(dict));
        self.dict_ready = true;
    }

    /// Dicionário pronto para o envio (negociado + HELLO completo).
    pub fn dict_ready(&self) -> bool {
        self.dict_ready
    }

    /// Publica o registro local (`HELLO`, §4.4) e devolve o registro do
    /// PEER. Obrigatório quando `caps::DICT` foi concedido: ambos os lados
    /// derivam o mesmo dicionário do registro do servidor antes do
    /// primeiro frame de trabalho — nenhum byte de dicionário cruza o fio.
    pub fn exchange_hello(
        &mut self,
        local: &[schema::DeviceDesc],
        timeout: Duration,
    ) -> Result<Vec<schema::DeviceDesc>, TransportError> {
        self.neg_seq = self.neg_seq.wrapping_add(1);
        let req = Message::hello(local.to_vec(), self.neg_seq);
        let resp = self.request(&req, timeout)?;
        if resp.opcode != schema::op::HELLO {
            return Err(TransportError::Broken(
                "resposta ao HELLO não é HELLO (§4.4)".into(),
            ));
        }
        let Body::Hello { devices } = resp.body else {
            return Err(TransportError::Broken(
                "corpo do HELLO inválido (§4.4)".into(),
            ));
        };
        Ok(devices)
    }

    // -----------------------------------------------------------------
    // v1.1 — Autenticação PSK do canal remoto (§4.6)
    // -----------------------------------------------------------------

    /// Handshake PSK: espera o `AUTH_CHALLENGE` do servidor, responde com
    /// nonce próprio + HMAC e exige o `AUTH_OK` com o mesmo seq. Falha em
    /// qualquer passo ⇒ erro (a conexão não deve ser usada).
    pub fn authenticate(&mut self, key: &[u8], timeout: Duration) -> Result<(), TransportError> {
        let challenge = self.receive(timeout)?;
        let Body::AuthChallenge { scheme, nonce } = challenge.body else {
            return Err(TransportError::Broken(
                "primeira mensagem do peer não é AUTH_CHALLENGE (§4.6)".into(),
            ));
        };
        if scheme != schema::AUTH_SCHEME_PSK_HMAC_SHA256 {
            return Err(TransportError::Broken(format!(
                "scheme de autenticação desconhecido: {scheme}"
            )));
        }
        let nonce_cliente =
            crate::auth::nonce().map_err(|e| TransportError::Broken(format!("RNG: {e}")))?;
        let mac = crate::auth::mac(key, &nonce_cliente, &nonce);
        let resp = self.request(
            &Message::auth_response(nonce_cliente, mac, challenge.seq),
            timeout,
        )?;
        if resp.opcode != schema::op::AUTH_OK {
            return Err(TransportError::Broken(
                "handshake PSK recusado pelo peer (chave errada?)".into(),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Servidor de referência (testes/integração)
// ---------------------------------------------------------------------------

/// Handle do servidor ativo; `Drop` sinaliza o encerramento e agrega a thread.
pub struct Server {
    pub identifier: String,
    desligar: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Sinaliza parada e espera a thread (o loop faz polling do flag).
    pub fn parar(mut self) {
        self.desligar.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }

    /// Construtor interno para servidores com loop próprio (`peer.rs`).
    pub(crate) fn from_parts(
        identifier: String,
        desligar: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    ) -> Self {
        Self {
            identifier,
            desligar,
            handle,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.desligar.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Servidor Unix: aceita conexões até `parar()`/`Drop`; **cada conexão é
/// servida em sua própria thread** (peers persistentes não bloqueiam novos
/// clientes). O `handler` devolve a resposta de cada pedido (`None` = sem
/// resposta — simula ator mudo para testar timeout).
pub fn serve_unix<F>(path: &Path, handler: F) -> Result<Server, TransportError>
where
    F: Fn(Message) -> Option<Message> + Clone + Send + 'static,
{
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?;
    let desligar = Arc::new(AtomicBool::new(false));
    let flag = desligar.clone();
    let identifier = format!("unix:{}", path.display());
    let path = path.to_path_buf();
    let handle = std::thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((flow, _)) => {
                    let handler = handler.clone();
                    let flag_conn = flag.clone();
                    std::thread::spawn(move || {
                        serve_connection(Current::Unix(flow), &handler, &flag_conn)
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
    Ok(Server {
        identifier,
        desligar,
        handle: Some(handle),
    })
}

/// Servidor TCP em porta efêmera; devolve a porta sorteada. Thread por
/// conexão, como no Unix.
pub fn serve_tcp<F>(handler: F) -> Result<(Server, u16), TransportError>
where
    F: Fn(Message) -> Option<Message> + Clone + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| TransportError::ConnectionFailed(format!("{e}")))?;
    let desligar = Arc::new(AtomicBool::new(false));
    let flag = desligar.clone();
    let identifier = format!("tcp:127.0.0.1:{port}");
    let handle = std::thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((flow, _)) => {
                    let handler = handler.clone();
                    let flag_conn = flag.clone();
                    std::thread::spawn(move || {
                        serve_connection(Current::Tcp(flow), &handler, &flag_conn)
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
    });
    Ok((
        Server {
            identifier,
            desligar,
            handle: Some(handle),
        },
        port,
    ))
}

fn serve_connection<F>(mut flow: Current, handler: &F, flag: &AtomicBool)
where
    F: Fn(Message) -> Option<Message>,
{
    flow.set_timeout(Duration::from_millis(250));
    let mut rest: Vec<u8> = Vec::new();
    loop {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        // Lê o próximo frame do fluxo (bloqueia até o timeout curto da conexão).
        match read_frame(&mut flow, &mut rest, None) {
            Ok(Some(msg)) => {
                if msg.opcode == schema::op::BYE {
                    return;
                }
                if let Some(resp) = handler(msg) {
                    let frame = match schema::encode_to_vec(&resp) {
                        Ok(f) => f,
                        Err(_) => return,
                    };
                    if flow.write_all(&frame).is_err() {
                        return;
                    }
                }
            }
            Ok(None) => continue, // frame incompleto; lê mais
            Err(TransportError::Timeout) => continue,
            Err(_) => return,
        }
    }
}

/// Extrai um frame de `rest` (buffer acumulado); devolve `None` se ainda
/// incompleto. Os bytes consumidos são removidos do buffer. O dicionário é
/// TIPADO (v1.3 §4.8): id 2 decodifica só com `DictConexao::Lz4`, id 3 só
/// com `DictConexao::Zstd` (o contrário é `UnknownCompression` — fail
/// closed por construção).
pub(crate) fn read_frame(
    flow: &mut Current,
    rest: &mut Vec<u8>,
    dict: Option<&schema::compress::DictConexao>,
) -> Result<Option<Message>, TransportError> {
    if rest.len() >= 4 {
        let total = schema::peek_frame_len(rest)?;
        if rest.len() >= total {
            let (msg, _) = schema::decode(rest)?;
            rest.drain(..total);
            return Ok(Some(msg));
        }
    }
    let mut buf = [0u8; 4096];
    let n = match flow.read(&mut buf) {
        Ok(0) => return Err(TransportError::Broken("fim do fluxo".into())),
        Ok(n) => n,
        Err(e)
            if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
        {
            return Err(TransportError::Timeout)
        }
        Err(e) => return Err(TransportError::Broken(format!("leitura: {e}"))),
    };
    rest.extend_from_slice(&buf[..n]);
    if rest.len() >= 4 {
        let total = schema::peek_frame_len(rest)?;
        if rest.len() >= total {
            let (msg, _) = schema::decode_with_conexao(rest, dict)?;
            rest.drain(..total);
            return Ok(Some(msg));
        }
    }
    Ok(None)
}

/// Espera o servidor aceitar conexões (poll de conectividade).
pub fn wait_ready_unix(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if UnixStream::connect(path).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}
