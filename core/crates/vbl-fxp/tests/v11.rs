//! E2E dos recursos v1.1 (docs/FXP-SCHEMA-v1.md §4.5–§4.8): negociação CAPS,
//! autenticação PSK, batching com honestidade por item, timestamps do fio e
//! compressão — sobre `PeerServer` (lado servidor) × `FxpBus` (lado cliente).
//!
//! Regra de ouro do plano: wire default é bit a bit v1.0 — todo recurso aqui
//! é opt-in nas duas pontas; os testes do `bus.rs` (sem recursos) continuam
//! passando intocados.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::bus::kinds;
use vbl_fxp::registry::{DeviceEntry, FxpConfig, OperationMode};
use vbl_fxp::schema::caps;
use vbl_fxp::transport::wait_ready_unix;
use vbl_fxp::{BusConfig, DeviceRegistry, FxpBus, PeerConfig, PeerServer, RemoteAddr};
use vbl_runtime::fxp::{ActOutcome, SensorFailure, Fxp, Value};
use vbl_runtime::ledger::ChainLedger;
use vbl_runtime::FxpSimulator;

const DEADLINE: Duration = Duration::from_secs(2);

fn tmpdir(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "vbl-fxp-v11-{}-{}-{}.sock",
        name,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Registro do PEER: três sensores simulados (temp_a/temp_b/temp_c) e um
/// registrado porém inacessível (temp_bad — rota real sem hardware, §4.7).
fn registry_peer() -> DeviceRegistry {
    let mut r = DeviceRegistry::new();
    for n in ["temp_a", "temp_b", "temp_c"] {
        let _ = r.register(DeviceEntry::sensor(n, "temperature", "°C", 1.0));
    }
    let mut ruim = DeviceEntry::sensor("temp_bad", "temperature", "°C", 1.0);
    ruim.mode = vbl_fxp::registry::DeviceMode::Real;
    ruim.endpoint = vbl_fxp::registry::Endpoint::Remote {
        addr: RemoteAddr::Unix(PathBuf::from("/nonexistent/vbl-teste.sock")),
    };
    let _ = r.register(ruim);
    r
}

/// Bus do PEER: híbrido ⇒ simulados roteiam, temp_bad fica inacessível.
fn peer_bus(reg: DeviceRegistry) -> FxpBus {
    FxpBus::build(
        reg,
        BusConfig { mode: OperationMode::Hybrid, ..Default::default() },
        FxpSimulator::new(),
    )
}

/// Registro do CLIENTE: todos os sensores do peer como endpoints remotos.
fn registry_cliente(sock: &std::path::Path, nomes: &[&str]) -> DeviceRegistry {
    let mut r = DeviceRegistry::new();
    let mut cfg = "mode = hibrido\ncache_ttl_ms = 0\n".to_string();
    for n in nomes {
        cfg.push_str(&format!("{n}.grandeza = temperatura\n{n}.unidade = C\n"));
        cfg.push_str(&format!("{n}.mode = real\n{n}.endpoint = unix:{}\n", sock.display()));
    }
    FxpConfig::parse(&cfg).unwrap().apply(&mut r).unwrap();
    r
}

fn bus_cliente(reg: DeviceRegistry, extra: impl FnOnce(&mut BusConfig)) -> FxpBus {
    let mut cfg = BusConfig { mode: OperationMode::Hybrid, ..Default::default() };
    extra(&mut cfg);
    FxpBus::build(reg, cfg, FxpSimulator::new())
}

fn contar(ledger: &ChainLedger, kind: &str) -> usize {
    ledger.events.iter().filter(|e| e.kind == kind).count()
}

#[test]
fn e2e_caps_batch_timestamp_compression_negociados() {
    let sock = tmpdir("completo");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: caps::LZ4 | caps::BATCH | caps::TIMESTAMP, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut bus = bus_cliente(
        registry_cliente(&sock, &["temp_a", "temp_b", "temp_c"]),
        |c| {
            c.batch_prefetch = true;
            c.compression = true;
            c.wire_timestamp = true;
        },
    );
    let mut ledger = ChainLedger::new();

    // CAPS concedidas = interseção cheia (as duas pontas anunciam tudo).
    let addr = RemoteAddr::Unix(sock.to_path_buf());
    let v = bus.read_sensor("temp_a", &mut ledger).unwrap();
    assert!(v.is_finite());
    assert_eq!(
        bus.granted_caps_of(&addr),
        caps::LZ4 | caps::BATCH | caps::TIMESTAMP
    );

    // Timestamp físico do fio (§5): anotação de laboratório, Some quando o
    // peer carimba — o relógio VIRTUAL dos eventos continua sendo do runtime.
    let ts = bus.wire_timestamp_of("temp_a");
    assert!(ts.is_some(), "peer anunciou TIMESTAMP; leitura deve vir carimbada");
    assert!(ts.unwrap() > 1_700_000_000_000_000, "µs desde o epoch plausível");

    // Prefetch (§4.7): temp_a miss ⇒ lote com os 3 vencidos ⇒ b/c pré-no cache.
    assert_eq!(contar(&ledger, kinds::FXP_BATCH), 1, "um lote, um evento");
    let v2 = bus.read_sensor("temp_b", &mut ledger).unwrap();
    assert!(v2.is_finite());
    assert_eq!(contar(&ledger, kinds::FXP_BATCH), 1, "temp_b veio do prefetch: sem novo lote");

    // Compressão (§4.8): negociada e em uso — nada quebra no roundtrip
    // (frames grandes comprimidos; pequenos viajam planos, decisão do codec).
    let v3 = bus.read_sensor("temp_c", &mut ledger).unwrap();
    assert!(v3.is_finite());
    assert_eq!(contar(&ledger, kinds::FXP_BATCH), 1);
}

#[test]
fn e2e_auth_psk_chave_certa_abre_errada_fecha() {
    let sock = tmpdir("auth");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig {
            psk: Some(b"chave-compartilhada".to_vec()),
            caps: caps::TIMESTAMP,
            ..Default::default()
        },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    // Chave certa: handshake transparente, leitura funciona.
    let mut ok = bus_cliente(
        registry_cliente(&sock, &["temp_a"]),
        |c| c.psk = Some(b"chave-compartilhada".to_vec()),
    );
    let mut ledger = ChainLedger::new();
    assert!(ok.read_sensor("temp_a", &mut ledger).is_ok());

    // Chave errada: fechamento sem AUTH_OK ⇒ falha honesta (nunca valor).
    let mut ruim = bus_cliente(
        registry_cliente(&sock, &["temp_a"]),
        |c| c.psk = Some(b"chave-errada".to_vec()),
    );
    let mut ledger2 = ChainLedger::new();
    assert!(matches!(
        ruim.read_sensor("temp_a", &mut ledger2),
        Err(SensorFailure::Inaccessible)
    ));
}

#[test]
fn e2e_cliente_sem_auth_contra_servidor_com_psk_falha_fechado() {
    let sock = tmpdir("semauth");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: Some(b"segredo".to_vec()), caps: 0, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    // Sem PSK configurada, o cliente não passa do gate de auth (§4.6):
    // nenhuma leitura — condição não avaliada, alerta no Caderno.
    let mut bus = bus_cliente(registry_cliente(&sock, &["temp_a"]), |_| {});
    let mut ledger = ChainLedger::new();
    assert!(matches!(
        bus.read_sensor("temp_a", &mut ledger),
        Err(SensorFailure::Inaccessible)
    ));
    assert_eq!(contar(&ledger, "ALERT"), 1, "falha de I/O registrada com alerta");
}

#[test]
fn e2e_falha_de_item_no_lote_so_alerta_quando_o_programa_pede() {
    let sock = tmpdir("lote");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: caps::BATCH, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut bus = bus_cliente(
        registry_cliente(&sock, &["temp_a", "temp_b", "temp_bad"]),
        |c| c.batch_prefetch = true,
    );
    let mut ledger = ChainLedger::new();

    // Primeira leitura: lote [temp_a, temp_b, temp_bad] — temp_bad falha no
    // peer, mas o programa não pediu temp_bad: SEM alerta (honestidade §4.7
    // do schema: o alerta pertence à pergunta feita).
    let v = bus.read_sensor("temp_a", &mut ledger).unwrap();
    assert!(v.is_finite());
    assert_eq!(contar(&ledger, "ALERT"), 0, "falha pré-buscada não é alertada");

    // Agora o programa pede temp_bad: cache-miss ⇒ READ individual ⇒ erro
    // honesto com alerta (condição não avaliada neste tick).
    assert!(matches!(
        bus.read_sensor("temp_bad", &mut ledger),
        Err(SensorFailure::Inaccessible)
    ));
    assert_eq!(contar(&ledger, "ALERT"), 1, "falha perguntada é alertada");
}

#[test]
fn e2e_peer_v1_0_degrada_com_evento_e_continua_operando() {
    // Servidor "v1.0": responde BYE a qualquer opcode desconhecido (CAPS não
    // existe lá). O cliente detecta, degrada para v1.0 puro e registra o
    // evento fxp_peer_v1 — nunca falha silenciosa, nunca deixa de operar.
    use vbl_fxp::transport::serve_unix;
    let sock = tmpdir("v10");
    let _srv = serve_unix(&sock, |msg| match msg.opcode {
        vbl_fxp::schema::op::READ => Some(vbl_fxp::Message::read_ok(42.0, "temp_a", false, msg.seq)),
        _ => Some(vbl_fxp::Message::bye(msg.seq)),
    })
    .expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut bus = bus_cliente(
        registry_cliente(&sock, &["temp_a"]),
        |c| {
            c.batch_prefetch = true;
            c.compression = true;
            c.wire_timestamp = true;
        },
    );
    let mut ledger = ChainLedger::new();
    let v = bus.read_sensor("temp_a", &mut ledger).unwrap();
    assert_eq!(v, 42.0);
    assert_eq!(contar(&ledger, kinds::FXP_PEER_V1), 1, "degradação logada");
    // Sem timestamp: peer v1.0 não carimba — ausência é honesta.
    assert_eq!(bus.wire_timestamp_of("temp_a"), None);
    // Segunda leitura segue v1.0 sem re-negociar a cada frame.
    let v2 = bus.read_sensor("temp_a", &mut ledger).unwrap();
    assert_eq!(v2, 42.0);
}

// ══════════════════════════════════════════════════════════════════════════
// v1.1 §4.9 — Descoberta multicast: `endpoint = discover:<identificador>`
// ══════════════════════════════════════════════════════════════════════════

fn multicast_ou_skip() -> bool {
    let g = vbl_fxp::discover::DEFAULT_GROUP;
    let ip = match g.ip() { std::net::IpAddr::V4(v) => v, _ => return false };
    std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|s| s.join_multicast_v4(&ip, &std::net::Ipv4Addr::UNSPECIFIED))
        .is_ok()
}

#[test]
fn e2e_descoberta_resolve_peer_no_build_e_sem_anuncio_inacessivel() {
    if !multicast_ou_skip() {
        println!("skip: multicast indisponível neste ambiente (§4.9, caminho honesto)");
        return;
    }
    use vbl_fxp::discover::{registry_hash, Announcer};
    use vbl_fxp::peer::serve_tcp_peer;
    use vbl_fxp::registry::Endpoint;

    // Peer TCP anunciando "fxpd-e2e-descoberto".
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: caps::TIMESTAMP, ..Default::default() },
    );
    let (_srv, port) = serve_tcp_peer(&peer).expect("servidor tcp");
    // Origem do beacon = interface de saída multicast; o servidor escuta em
    // 0.0.0.0 (serve_tcp_peer), então o IP anunciado é conectável.
    let _ann = Announcer::start(
        "fxpd-e2e-descoberto",
        port,
        registry_hash(&["temp_a".into()]),
        vbl_fxp::discover::DEFAULT_GROUP,
        Duration::from_millis(50),
    )
    .expect("anunciante");

    // Bus do cliente com endpoint `discover:` — resolvido no build().
    let mut r = DeviceRegistry::new();
    let cfg = "mode = hibrido\ncache_ttl_ms = 0\n\
               temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
               temp_a.mode = real\ntemp_a.endpoint = discover:fxpd-e2e-descoberto\n";
    FxpConfig::parse(cfg).unwrap().apply(&mut r).unwrap();
    let mut bus = bus_cliente(r, |c| {
        c.discover_window = Duration::from_millis(500);
        c.wire_timestamp = true;
    });
    let mut ledger = ChainLedger::new();
    assert!(bus.read_sensor("temp_a", &mut ledger).is_ok(), "descoberto ⇒ leitura funciona");
    assert!(bus.wire_timestamp_of("temp_a").is_some());

    // Endpoint descritível de volta (probe/config).
    let mut r2 = DeviceRegistry::new();
    FxpConfig::parse(&format!(
        "mode = hibrido\ntemp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
               temp_a.mode = real\ntemp_a.endpoint = discover:fxpd-nunca-anunciado-{}\n",
        std::process::id()
    ))
    .unwrap()
    .apply(&mut r2)
    .unwrap();
    let mut bus2 = bus_cliente(r2, |c| { c.discover_window = Duration::from_millis(150); });
    // Sem anúncio no prazo: registrado porém inacessível (FORMAL §4.7) —
    // construção NUNCA falha.
    let entrada = bus2.registry_rico().devices().find(|d| d.name == "temp_a").unwrap();
    assert!(matches!(entrada.endpoint, Endpoint::AutoRemote { .. }));
    assert_eq!(Endpoint::AutoRemote { identifier: "x".into() }.description(), "discover:x");
    let mut led2 = ChainLedger::new();
    assert!(matches!(
        bus2.read_sensor("temp_a", &mut led2),
        Err(SensorFailure::Inaccessible)
    ));
}

// ══════════════════════════════════════════════════════════════════════════
// v1.1 — ACT remota (§4.3 via PeerServer): entregue, rejeitada por limite,
// ator ausente e fallback executado — tudo espelhado em ACT_ACK.
// ══════════════════════════════════════════════════════════════════════════

fn registry_peer_com_ator() -> DeviceRegistry {
    use vbl_runtime::fxp::ActorLimits;
    let mut r = registry_peer();
    // Ator do peer com limites: 10..255, safety 200.
    let _ = r.register(DeviceEntry::actor(
        "Fan",
        ActorLimits { min: Some(10.0), max: Some(255.0), safety_limit: Some(200.0) },
    ));
    // Ator de fallback citável (ReserveFan).
    let _ = r.register(DeviceEntry::actor(
        "ReserveFan",
        ActorLimits { min: Some(10.0), max: Some(255.0), safety_limit: None },
    ));
    // Ator de extensão com min > 0: o sim do peer NÃO o pré-registra,
    // então os limites do registro rico valem (min 50/max 100).
    let _ = r.register(DeviceEntry::actor(
        "Servo",
        ActorLimits { min: Some(50.0), max: Some(100.0), safety_limit: None },
    ));
    // Ator que FALHA no peer (endpoint morto) — dispara fallback.
    let mut moribundo = DeviceEntry::actor(
        "Dying",
        ActorLimits { min: Some(0.0), max: Some(255.0), safety_limit: None },
    );
    moribundo.mode = vbl_fxp::registry::DeviceMode::Real;
    moribundo.endpoint = vbl_fxp::registry::Endpoint::Remote {
        addr: RemoteAddr::Unix(PathBuf::from("/nonexistent/vbl-teste.sock")),
    };
    let _ = r.register(moribundo);
    r
}

#[test]
fn e2e_act_remota_entrega_rejeita_e_aplica_fallback() {
    let sock = tmpdir("act-remota");
    // Política de fallback Fan = ReserveFan no PEER (quem atua decide).
    let mut reg = registry_peer_com_ator();
    FxpConfig::parse("fallback.Fan = ReserveFan\nfallback.Dying = Fan\n")
        .unwrap()
        .apply(&mut reg)
        .unwrap();
    let peer = PeerServer::new(
        peer_bus(reg),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: caps::TIMESTAMP, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut reg_cli = registry_cliente(&sock, &[]);
    // O cliente também precisa conhecer os atores (FORMAL §6: registro local
    // valida antes de enviar) — endpoints remotos.
    let mut cfg = format!(
        "mode = hibrido\ncache_ttl_ms = 0\n\
         Fan.mode = real\nFan.endpoint = unix:{}\nFan.min = 10\nFan.max = 255\nFan.safety_limit = 200\n\
         ReserveFan.mode = real\nReserveFan.endpoint = unix:{}\nReserveFan.min = 10\nReserveFan.max = 255\n\
         Dying.mode = real\nDying.endpoint = unix:{}\nDying.min = 0\nDying.max = 255\n",
        sock.display(),
        sock.display(),
        sock.display()
    );
    cfg.push_str("fallback.Fan = ReserveFan\n");
    FxpConfig::parse(&cfg)
        .unwrap()
        .apply(&mut reg_cli)
        .unwrap();
    let mut bus = bus_cliente(reg_cli, |c| { c.wire_timestamp = true; });
    let mut ledger = ChainLedger::new();

    // 1) Entregue: valor dentro dos limites.
    assert_eq!(
        bus.act("Fan", Value::Num(50.0), &mut ledger),
        ActOutcome::Delivered
    );

    // 2) Rejeitada pelo limite do registro (sem envio): safety 200.
    assert!(matches!(
        bus.act("Fan", Value::Num(220.0), &mut ledger),
        ActOutcome::Rejected { .. }
    ));

    // 3) Ator ausente nos DOIS lados.
    assert_eq!(
        bus.act("Ninguem", Value::Num(1.0), &mut ledger),
        ActOutcome::MissingActor
    );

    // 4) Primário falha no PEER ⇒ fallback executado lá e espelhado no ACK.
    let outcome = bus.act("Dying", Value::Num(42.0), &mut ledger);
    assert!(
        matches!(outcome, ActOutcome::FallbackExecuted { .. }),
        "esperava FallbackExecuted, veio {outcome:?}"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// PeerServer × cliente cru: HELLO, HEARTBEAT, opcode desconhecido e o lote
// de 1 (que volta a READ individual sem ganho — §4.7).
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn peer_responde_hello_heartbeat_e_ignora_desconhecido() {
    use vbl_fxp::schema::Body;
    use vbl_fxp::transport::Connection;
    use vbl_fxp::Message;

    let sock = tmpdir("peer-cru");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig::default(),
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut c = Connection::unix(&sock, Duration::from_secs(2)).expect("conectar");

    // HELLO devolve o registro DO servidor (§4.4) — com DeviceDesc preenchido.
    let resp = c.request(&Message::hello(Vec::new(), 1), Duration::from_secs(2)).expect("hello");
    let Body::Hello { devices, .. } = resp.body else {
        panic!("esperava HELLO_OK, veio {:?}", resp.opcode)
    };
    assert!(devices.iter().any(|d| d.name() == "temp_a"), "{devices:?}");

    // HEARTBEAT responde ack de vida.
    let resp = c
        .request(&Message::heartbeat("temp_a", 2), Duration::from_secs(2))
        .expect("heartbeat");
    assert_eq!(resp.seq, 2);

    // Opcode desconhecido (0x7F): o peer fecha a conexão (§5 — fio estrito).
    // Monta um frame válido com opcode trocado: encode de READ + patch opcode.
    let mut frame = vbl_fxp::schema::encode_to_vec(&Message::read("temp_a", 3, true)).unwrap();
    frame[4 + 4] = 0x7F;
    use std::io::{Read, Write};
    let mut s = std::os::unix::net::UnixStream::connect(&sock).expect("conectar cru");
    s.write_all(&frame).unwrap();
    s.flush().unwrap();
    let mut buf = [0u8; 16];
    let n = s.read(&mut buf).unwrap_or(0);
    assert_eq!(n, 0, "peer deve fechar diante de opcode desconhecido (recebeu {n} bytes)");
}

#[test]
fn batch_de_um_sensor_cai_no_caminho_individual() {
    // Peer com UM sensor remoto no endereço: lote de 1 = READ individual.
    let sock = tmpdir("batch-1");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: caps::BATCH, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    // Cliente com apenas temp_b remota (temp_a/temp_c fora do registro →
    // alvos_de_batch devolve só temp_b = 1 item).
    let reg = registry_cliente(&sock, &["temp_b"]);
    let mut bus = bus_cliente(reg, |c| { c.batch_prefetch = true; });
    let mut ledger = ChainLedger::new();
    assert!(bus.read_sensor("temp_b", &mut ledger).is_ok());
    // Sem evento de lote: 1 item = caminho individual (§4.7).
    assert_eq!(contar(&ledger, kinds::FXP_BATCH), 0);
}

// ══════════════════════════════════════════════════════════════════════════
// Ramos restantes: rejeição por min/max no fio, act com prioridade, valor
// Ident, sensor no lugar de ator, item de lote com erro do peer (temp_bad).
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn e2e_act_ack_rejeicao_por_minimo_maximo_e_outros_acks() {
    let sock = tmpdir("act-acks");
    let peer = PeerServer::new(
        peer_bus(registry_peer_com_ator()),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: caps::TIMESTAMP, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut reg = registry_cliente(&sock, &[]);
    // Cliente com limites FOLGADOS (0..1000): quem rejeita por min/max é o
    // registro do PEER (10..255, safety 200) — o ACK volta tipado.
    let cfg = format!(
        "mode = hibrido\ncache_ttl_ms = 0\n\
         Fan.mode = real\nFan.endpoint = unix:{}\nFan.min = 0\nFan.max = 1000\n\
         temp_a.mode = real\ntemp_a.endpoint = unix:{}\n",
        sock.display(),
        sock.display()
    );
    FxpConfig::parse(&cfg).unwrap().apply(&mut reg).unwrap();

    // No registro do cliente, Servo precisa existir (endpoint remoto, limites
    // folgados): quem rejeita é o PEER, e o ACK volta tipado.
    let cfg2 = format!(
        "Servo.mode = real\nServo.endpoint = unix:{}\nServo.min = 0\nServo.max = 1000\n",
        sock.display()
    );
    FxpConfig::parse(&cfg2).unwrap().apply(&mut reg).unwrap();
    let mut bus = bus_cliente(reg, |c| { c.wire_timestamp = true; });
    let mut ledger = ChainLedger::new();

    // Abaixo do MÍNIMO do peer ⇒ ACT_ACK Rejected(limit=Min) mapeado de volta.
    assert!(matches!(
        bus.act("Servo", Value::Num(5.0), &mut ledger),
        ActOutcome::Rejected { limit: vbl_runtime::fxp::Limit::Min, .. }
    ));
    // Acima do MÁXIMO do peer ⇒ Rejected(Max).
    assert!(matches!(
        bus.act("Servo", Value::Num(300.0), &mut ledger),
        ActOutcome::Rejected { limit: vbl_runtime::fxp::Limit::Max, .. }
    ));
    // Valor Ident (não numérico) atravessa o fio e volta do peer.
    let _ = bus.act("Fan", Value::Ident("auto".into()), &mut ledger);
    // Atuar num SENSOR (limites de ator inexistentes no registro do peer).
    let _ = bus.act("temp_a", Value::Num(1.0), &mut ledger);

    // act_with_priority: violação local rejeitada SEM envio; ator
    // desconhecido ⇒ MissingActor honesto.
    assert!(matches!(
        bus.act_with_priority("Fan", Value::Num(5000.0), 0, &mut ledger),
        ActOutcome::Rejected { .. }
    ));
    assert_eq!(
        bus.act_with_priority("Ninguem", Value::Num(1.0), 0, &mut ledger),
        ActOutcome::MissingActor
    );

    // invalidate_cache (API de probe/bench) limpa o cache de verdade.
    assert!(bus.read_sensor("temp_a", &mut ledger).is_ok());
    bus.invalidate_cache();
    assert!(bus.wire_timestamp_of("temp_a").is_some());
    assert!(bus.read_sensor("temp_a", &mut ledger).is_ok());
}

#[test]
fn e2e_item_de_lote_com_erro_do_peer_so_alerta_quando_perguntado() {
    let sock = tmpdir("batch-err-item");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: caps::BATCH, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    // Cliente com temp_a (ok) e temp_bad (peer responde READ_ERR: endpoint
    // morto) — as duas no MESMO endereço ⇒ lote de 2 com 1 item errado.
    let mut reg = registry_cliente(&sock, &["temp_a"]);
    let cfg = format!(
        "temp_bad.mode = real\ntemp_bad.endpoint = unix:{}\ntemp_bad.grandeza = temperatura\ntemp_bad.unidade = C\n",
        sock.display()
    );
    FxpConfig::parse(&cfg).unwrap().apply(&mut reg).unwrap();
    let mut bus = bus_cliente(reg, |c| { c.batch_prefetch = true; });
    let mut ledger = ChainLedger::new();

    // 1ª pergunta: temp_bad (o lote leva temp_a + temp_bad = 2 itens). O peer
    // responde Err para temp_bad ⇒ alerta honesto com o motivo espelhado.
    let r = bus.read_sensor("temp_bad", &mut ledger);
    assert!(matches!(r, Err(SensorFailure::Inaccessible)));
    assert!(ledger.events.iter().any(|e| e.kind == "ALERT"));
    assert_eq!(contar(&ledger, kinds::FXP_BATCH), 1);

    // 2ª pergunta: temp_a ainda sem cache ⇒ lote novo de... temp_a só
    // (temp_bad ficou inacessível) e volta OK.
    assert!(bus.read_sensor("temp_a", &mut ledger).is_ok());
    assert!(
        ledger.events.iter().all(|e| e.kind != "ALERT"
            || !e.msg.contains("temp_a")),
        "temp_a acessível não deve virar alerta: {:?}",
        ledger.events.iter().filter(|e| e.kind == "ALERT").collect::<Vec<_>>()
    );
}

#[test]
fn peer_fecha_conexao_com_lixo_pre_autenticacao() {
    use vbl_fxp::Message;
    // PSK obrigatória: o primeiro frame TEM de ser AUTH_RESPONSE; lixo ⇒
    // conexão fechada sem resposta (§4.6 — fail-closed).
    use std::io::{Read, Write};
    let sock = tmpdir("psk-lixo");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: Some(b"segredo".to_vec()), caps: 0, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    // O servidor FALA PRIMEIRO (challenge de 50 B); drena-o antes de testar
    // o portão pré-auth.
    let frame = vbl_fxp::schema::encode_to_vec(&Message::read("temp_a", 1, true)).unwrap();
    let mut s = std::os::unix::net::UnixStream::connect(&sock).expect("conectar");
    let mut challenge = [0u8; 4 + 12 + 2 + 32];
    s.read_exact(&mut challenge).expect("challenge do peer");
    assert_eq!(&challenge[4..7], b"FXP", "primeiro frame deve ser o challenge");

    // READ comum antes do PSK ⇒ o peer fecha sem responder (fail-closed).
    s.write_all(&frame).unwrap();
    s.flush().unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(s.read(&mut buf).unwrap_or(0), 0, "peer deve fechar sem PSK");

    // Bytes NO MEIO do frame (length mentiroso): fecha também.
    let mut s2 = std::os::unix::net::UnixStream::connect(&sock).expect("conectar 2");
    let mut challenge2 = [0u8; 4 + 12 + 2 + 32];
    s2.read_exact(&mut challenge2).expect("challenge 2");
    s2.write_all(&[0xFF, 0xFF, 0xFF, 0x7F, b'F']).unwrap();
    s2.flush().unwrap();
    let mut buf2 = [0u8; 16];
    assert_eq!(s2.read(&mut buf2).unwrap_or(0), 0);
}

#[test]
fn peer_cru_responde_lote_com_opcode_errado_e_bus_falha_honesto() {
    use vbl_fxp::schema::op;
    use vbl_fxp::transport::serve_unix;
    use vbl_fxp::Message;
    // Servidor genérico: aceita CAPS, e ao READ_BATCH responde HELLO
    // (opcode errado) — o bus deve fechar e alertar, nunca confiar.
    let dir = std::env::temp_dir().join(format!("rogue-batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("fxpd.sock");
    let _srv = serve_unix(&sock, |msg| {
        if msg.opcode == op::CAPS {
            Some(Message::caps_ok(0b11, msg.seq)) // anuncia BATCH
        } else {
            Some(Message::hello(Vec::new(), msg.seq)) // LIXO para o lote
        }
    })
    .expect("servidor genérico");

    let mut reg = registry_cliente(&sock, &["temp_a"]);
    let cfg = format!(
        "temp_c.mode = real\ntemp_c.endpoint = unix:{}\n",
        sock.display()
    );
    FxpConfig::parse(&cfg).unwrap().apply(&mut reg).unwrap();
    let mut bus = bus_cliente(reg, |c| { c.batch_prefetch = true; });
    let mut ledger = ChainLedger::new();

    let r = bus.read_sensor("temp_a", &mut ledger);
    assert!(matches!(r, Err(SensorFailure::Inaccessible)), "veio {r:?}");
    assert!(ledger.events.iter().any(|e| e.kind == "ALERT"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn peer_batch_item_nao_registrado_viaja_e_vira_alerta_tipado() {
    // Cliente enxerga "temp_fantasma" como remota, mas o PEER não a conhece:
    // o lote volta com o item Err(nao_registrado) — tag 4 do fio (§4.7).
    let sock = tmpdir("batch-fantasma");
    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: caps::BATCH, ..Default::default() },
    );
    assert_eq!(peer.config().caps, caps::BATCH, "acessor de config honesto");
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut reg = registry_cliente(&sock, &["temp_a"]);
    let cfg = format!(
        "temp_fantasma.grandeza = temperatura\ntemp_fantasma.unidade = C\n\
         temp_fantasma.mode = real\ntemp_fantasma.endpoint = unix:{}\n",
        sock.display()
    );
    FxpConfig::parse(&cfg).unwrap().apply(&mut reg).unwrap();
    let mut bus = bus_cliente(reg, |c| { c.batch_prefetch = true; });
    let mut ledger = ChainLedger::new();

    // O PEER não conhece o nome ⇒ item Err(nao_registrado) espelhado de
    // volta como condição NÃO AVALIADA (§4.1/§4.7) — não é "inacessível".
    let r = bus.read_sensor("temp_fantasma", &mut ledger);
    assert!(matches!(r, Err(SensorFailure::NotRegistered)), "veio {r:?}");
    assert!(ledger
        .events
        .iter()
        .any(|e| e.kind == "ALERT" && e.msg.contains("temp_fantasma")));
}

#[test]
fn peer_act_com_str_e_rejeicao_de_safety_remota() {
    let sock = tmpdir("peer-act-str");
    // Peer com Servo de safety 90: 95 passa pelo cliente (max 1000) e é
    // rejeitado PELO PEER com limit=SafetyLimit (ack tag 2).
    let mut reg_peer = registry_peer_com_ator();
    let _ = reg_peer.register(DeviceEntry::actor(
        "Guincho",
        vbl_runtime::fxp::ActorLimits { min: Some(0.0), max: Some(100.0), safety_limit: Some(90.0) },
    ));
    let peer = PeerServer::new(
        peer_bus(reg_peer),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: 0, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut reg = registry_cliente(&sock, &[]);
    let cfg = format!(
        "mode = hibrido\n\
         Guincho.mode = real\nGuincho.endpoint = unix:{}\nGuincho.min = 0\nGuincho.max = 1000\n\
         StatusLed.mode = real\nStatusLed.endpoint = unix:{}\n",
        sock.display(),
        sock.display()
    );
    FxpConfig::parse(&cfg).unwrap().apply(&mut reg).unwrap();
    let mut bus = bus_cliente(reg, |_| {});
    let mut ledger = ChainLedger::new();

    // Rejeição de SAFETY que só o peer conhece (tag 2 no fio).
    assert!(matches!(
        bus.act("Guincho", Value::Num(95.0), &mut ledger),
        ActOutcome::Rejected { limit: vbl_runtime::fxp::Limit::SafetyLimit, .. }
    ));
    // WireValue::Str atravessa o fio e volta como ack tipado do peer.
    let out = bus.act("Guincho", Value::Str("modo".into()), &mut ledger);
    assert!(matches!(
        out,
        ActOutcome::Delivered
            | ActOutcome::Rejected { .. }
            | ActOutcome::InvalidValue { .. }
    ));
}

#[test]
fn hello_do_peer_carrega_a_matriz_completa_de_descritores() {
    use vbl_fxp::schema::DeviceDesc;
    use vbl_fxp::Message;
    // Peer com sensor COM limites, sensor SEM limites, ator COM safety e
    // ator sem limites: o HELLO codifica e o cliente decodifica os 4 sabores.
    let sock = tmpdir("hello-matriz");
    let mut reg = registry_peer_com_ator();
    let _ = reg.register(DeviceEntry::sensor("temp_nua", "temperature", "K", 0.0)); // sem min/max
    let _ = reg.register(DeviceEntry::actor(
        "Led",
        vbl_runtime::fxp::ActorLimits::default(), // ator sem limites
    ));
    let peer = PeerServer::new(
        peer_bus(reg),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: 0, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut c = vbl_fxp::transport::Connection::unix(&sock, Duration::from_secs(2)).expect("conectar");
    let resp = c.request(&Message::hello(Vec::new(), 1), Duration::from_secs(2)).expect("hello");
    let vbl_fxp::schema::Body::Hello { devices } = resp.body else {
        panic!("esperava HELLO_OK");
    };
    let nomes: Vec<&str> = devices.iter().map(|d| d.name()).collect();
    for esperado in ["temp_a", "temp_nua", "Fan", "Led"] {
        assert!(nomes.contains(&esperado), "faltou {esperado} em {nomes:?}");
    }
    // Os sabores voltam tipados pelo peer real: ator com limites+safety e
    // dispositivos sem limites (o sensor com faixa já coberto no codec).
    let ator_com_limites = devices.iter().any(|d| matches!(d,
        DeviceDesc::Actor { min: Some(_), max: Some(_), safety: Some(_), .. }));
    let sem_limites = devices.iter().any(|d| matches!(d,
        DeviceDesc::Sensor { min: None, max: None, .. } | DeviceDesc::Actor { min: None, max: None, .. }));
    assert!(ator_com_limites && sem_limites, "{devices:?}");
}

#[test]
fn peer_tcp_inacessivel_alerta_sem_travar() {
    // Endpoint tcp apontando para porta fechada: falha de conexão vira
    // sensor_inaccessible com motivo honesto (§4.7).
    let mut reg = registry_cliente(std::path::Path::new("/tmp/vbl-nunca.sock"), &["temp_a"]);
    let cfg = "cpu_extra.grandeza = temperature\ncpu_extra.unidade = C\n\
               cpu_extra.mode = real\ncpu_extra.endpoint = tcp:127.0.0.1:1\n";
    FxpConfig::parse(cfg).unwrap().apply(&mut reg).unwrap();
    let mut bus = bus_cliente(reg, |_| {});
    let mut ledger = ChainLedger::new();
    let r = bus.read_sensor("cpu_extra", &mut ledger);
    assert!(matches!(r, Err(SensorFailure::Inaccessible)), "veio {r:?}");
}

#[test]
fn peer_que_fecha_apos_caps_reconecta_e_falha_honesto() {
    // Sem PSK: cliente envia CAPS; o peer hostil fecha. O cliente remove a
    // conexão e reconecta (ramo de reconexão); o turno falha tipado.
    let sock = tmpdir("rogue-caps");
    use vbl_fxp::transport::serve_unix;
    let _srv = serve_unix(&sock, |_msg| None).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let reg = registry_cliente(&sock, &["temp_a"]);
    let mut bus = bus_cliente(reg, |_| {});
    let mut ledger = ChainLedger::new();
    let r = bus.read_sensor("temp_a", &mut ledger);
    assert!(matches!(r, Err(SensorFailure::Inaccessible)), "veio {r:?}");
}

#[test]
fn lote_de_um_sensor_cai_no_caminho_individual() {
    // Prefetch ligado com UM sensor remoto: lote de 1 não compensa —
    // mantém o caminho v1.0 (READ individual) e a leitura segue correta.
    let sock = tmpdir("lote-um");
    let mut reg = registry_peer();
    let _ = reg.register(DeviceEntry::sensor("temp_solo", "temperature", "°C", 1.0));
    let peer = PeerServer::new(
        peer_bus(reg),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: 0, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let regc = registry_cliente(&sock, &["temp_solo"]);
    let mut bus = bus_cliente(regc, |c| c.batch_prefetch = true);
    let mut ledger = ChainLedger::new();
    let v = bus.read_sensor("temp_solo", &mut ledger).expect("leitura");
    assert!(v.is_finite(), "leitura não finita: {v}");
}

#[test]
fn rogue_lote_sem_o_sensor_pedido_e_resposta_trocada_alertam() {
    use vbl_fxp::Message;
    use vbl_fxp::schema::{op, BatchResult};
    use vbl_fxp::transport::serve_unix;
    // Três roubos de lote, três cláusulas honestas do cliente (§4.7):
    // (a) resposta que não traz o sensor pedido ⇒ "lote sem o sensor pedido";
    // (b) resposta trocada (READ_OK no lugar do lote) ⇒ "resposta inesperada";
    // (c) peer some no meio do lote ⇒ "transporte".
    for truque in ["sem_nome", "resposta_trocada", "some"] {
        let sock = tmpdir(&format!("rogue-lote-{truque}"));
        let _srv = serve_unix(&sock, move |msg| {
            match msg.opcode {
                op::CAPS => Some(Message::caps_ok(0b010, msg.seq)), // só BATCH
                op::HELLO => Some(Message::hello(Vec::new(), msg.seq)),
                op::READ_BATCH => match truque {
                    "sem_nome" => Some(Message::read_batch_ok(
                        vec![BatchResult::Ok { value: 1.0, canonical: "temp_a".into() }],
                        msg.seq,
                    )),
                    "resposta_trocada" => Some(Message::read_ok(9.0, "temp_a", true, msg.seq)),
                    _ => {
                        // lote vazio é recusado pelo codec ⇒ o servidor morre e
                        // o cliente vê o fim do fluxo (cláusula de transporte).
                        Some(Message::read_batch_ok(Vec::new(), msg.seq))
                    }
                },
                _ => Some(Message::heartbeat_ack(true, msg.seq)),
            }
        })
        .expect("servidor");
        assert!(wait_ready_unix(&sock, DEADLINE));

        let reg = registry_cliente(&sock, &["temp_a", "temp_b"]);
        let mut bus = bus_cliente(reg, |c| c.batch_prefetch = true);
        let mut ledger = ChainLedger::new();
        let alvo = if truque == "sem_nome" { "temp_b" } else { "temp_a" };
        let r = bus.read_sensor(alvo, &mut ledger);
        assert!(matches!(r, Err(SensorFailure::Inaccessible)), "{truque}: {r:?}");
        assert!(
            ledger
                .events
                .iter()
                .any(|e| e.msg.contains("lote") || e.msg.contains("transporte")),
            "{truque}: sem alerta honesto — {:?}",
            ledger.events.iter().map(|e| e.msg.clone()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn violacao_de_limite_do_cliente_bloqueia_antes_do_fio() {
    use vbl_runtime::fxp::ActorLimits;
    // Cliente com limites MAIS APERTADOS que o peer: a rejeição local
    // (Min/Max) acontece antes de qualquer frame sair (§4.3).
    let sock = tmpdir("limite-cliente");
    let mut regp = registry_peer_com_ator();
    let _ = regp.register(DeviceEntry::actor(
        "Guincho",
        ActorLimits { min: Some(0.0), max: Some(100.0), safety_limit: Some(90.0) },
    ));
    let peer = PeerServer::new(
        peer_bus(regp),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: 0, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let cfg = format!(
        "temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
         temp_a.mode = real\ntemp_a.endpoint = unix:{}\n\
         Guincho.mode = real\nGuincho.endpoint = unix:{}\n\
         Guincho.min = 200\nGuincho.max = 1000\n",
        sock.display(),
        sock.display()
    );
    let mut regc = DeviceRegistry::new();
    FxpConfig::parse(&cfg).unwrap().apply(&mut regc).unwrap();
    let mut bus = bus_cliente(regc, |c| c.batch_prefetch = false);
    let mut ledger = ChainLedger::new();
    let out = bus.act("Guincho", Value::Num(95.0), &mut ledger);
    assert!(
        matches!(&out, ActOutcome::Rejected { .. }),
        "esperava rejeição local Min/Max, veio {out:?}"
    );
}

#[test]
fn ator_e_sensor_desconhecidos_falham_com_evento_no_caderno() {
    // Leitura sem rota (sensor fora do registro) e atuação em ator
    // desconhecido: ambos registrados com evento próprio (§4.7).
    let mut bus = bus_cliente(
        registry_cliente(std::path::Path::new("/tmp/vbl-nada.sock"), &["temp_a"]),
        |_| {},
    );
    let mut ledger = ChainLedger::new();
    let r = bus.read_sensor("fantasma", &mut ledger);
    assert!(matches!(r, Err(SensorFailure::NotRegistered)), "{r:?}");
    let out = bus.act("Ninguem", Value::Num(1.0), &mut ledger);
    assert!(matches!(out, ActOutcome::MissingActor), "{out:?}");
}

#[test]
fn power_real_ausente_marca_inacessivel_e_fallback_local_entrega() {
    // cpu_power com rota real (RAPL inexistente): a próxima operação com
    // caderno atualiza o retrato de potência e marca a potência como
    // inacessível — sem inventar número (§4.7).
    let sock = tmpdir("power-real");
    let cfg = format!(
        "temp_a.grandeza = temperatura\ntemp_a.unidade = C\n\
         temp_a.mode = real\ntemp_a.endpoint = unix:{}\n\
         cpu_power.grandeza = power\ncpu_power.unidade = W\n\
         cpu_power.mode = real\ncpu_power.endpoint = rapl_energy:/não/existe\n\
         ReserveFan.grandeza = power\nReserveFan.unidade = W\n",
        sock.display()
    );
    let mut reg = DeviceRegistry::new();
    FxpConfig::parse(&cfg).unwrap().apply(&mut reg).unwrap();

    let peer = PeerServer::new(
        peer_bus(registry_peer()),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: 0, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut bus = bus_cliente(reg, |c| c.batch_prefetch = false);
    let mut ledger = ChainLedger::new();
    // A leitura segue válida: cpu_power inacessível é retrato ausente, não
    // erro de leitura (§4.7 — a potência só fica indisponível p/ atribuição).
    let v = bus.read_sensor("temp_a", &mut ledger).expect("leitura com power ausente");
    assert!(v.is_finite(), "{v}");

    // Fan com rota remota morta + fallback local ReserveFan: a entrega cai
    // para o alternativo e é auditada como FallbackExecuted (§4.3).
    let cfg_fan = "Fan.mode = real\nFan.endpoint = tcp:127.0.0.1:1\n\
         Fan.min = 0\nFan.max = 255\n\
         ReserveFan.min = 0\nReserveFan.max = 255\n\
         fallback.Fan = ReserveFan\n";
    let mut reg2 = DeviceRegistry::new();
    FxpConfig::parse(cfg_fan).unwrap().apply(&mut reg2).unwrap();
    let mut bus2 = bus_cliente(reg2, |_| {});
    let mut ledger2 = ChainLedger::new();
    let out = bus2.act("Fan", Value::Num(50.0), &mut ledger2);
    assert!(
        matches!(&out, ActOutcome::FallbackExecuted { alternativo } if alternativo == "ReserveFan"),
        "esperava fallback para ReserveFan, veio {out:?}"
    );
}

#[test]
fn peer_cru_cumpre_o_protocolo_nas_bordas() {
    use vbl_fxp::Message;
    use vbl_fxp::schema::{op, Body};

    let sock = tmpdir("peer-bordas");
    let mut reg = registry_peer_com_ator();
    // registro grande ⇒ resposta HELLO acima do threshold de compressão.
    for i in 0..40 {
        let _ = reg.register(DeviceEntry::sensor(
            &format!("sensor_grande_{i:02}"),
            "temperature",
            "°C",
            1.0,
        ));
    }
    let peer = PeerServer::new(
        peer_bus(reg),
        ChainLedger::new(),
        PeerConfig { psk: None, caps: 0b111, ..Default::default() },
    );
    let _srv = vbl_fxp::peer::serve_unix_peer(&peer, &sock).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    // (1) READ_BATCH sem a capacidade BATCH negociada: o peer fecha sem
    // responder (violação de protocolo, §4.5).
    let mut c =
        vbl_fxp::transport::Connection::unix(&sock, Duration::from_secs(1)).expect("conectar");
    let resp = c.request(&Message::read_batch(vec!["temp_a".into()], 1), Duration::from_millis(300));
    assert!(resp.is_err(), "batch sem caps devia ser ignorado, veio {resp:?}");
    drop(c);

    // (2) ACT com corpo de outro opcode (frame forjado): peer fecha honesto.
    let mut c2 =
        vbl_fxp::transport::Connection::unix(&sock, Duration::from_secs(1)).expect("conectar 2");
    let forjado = Message {
        opcode: op::ACT,
        flags: 0,
        timestamp_us: None,
        seq: 2,
        name: "Fan".into(),
        body: Body::ReadOk { value: 1.0, canonical: "Fan".into() },
    };
    let _ = c2.enviar(&forjado);
    let resp2 = c2.receive(Duration::from_millis(300));
    assert!(resp2.is_err(), "ACT forjado devia fechar a conexão, veio {resp2:?}");
    drop(c2);

    // (3) HELLO negociado com LZ4 ⇒ resposta parte comprimida (§4.8).
    let mut c3 =
        vbl_fxp::transport::Connection::unix(&sock, Duration::from_secs(1)).expect("conectar 3");
    let caps = c3
        .request(&Message::caps(0b111, 3), Duration::from_secs(1))
        .expect("negociar");
    let _ = caps;
    let hello = c3
        .request(&Message::hello(Vec::new(), 4), Duration::from_secs(1))
        .expect("hello comprimido");
    assert!(hello.name.is_empty(), "{hello:?}");

    // (4) BYE encerra a conexão sem resposta (§4.2).
    let mut c4 =
        vbl_fxp::transport::Connection::unix(&sock, Duration::from_secs(1)).expect("conectar 4");
    c4.enviar(&Message::bye(5)).expect("enviar bye");
    let depois = c4.receive(Duration::from_millis(300));
    assert!(depois.is_err(), "pós-BYE a conexão devia terminar: {depois:?}");
}
