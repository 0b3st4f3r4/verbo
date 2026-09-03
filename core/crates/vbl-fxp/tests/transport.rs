//! Transporte Unix/TCP do schema v1.1: roundtrip, ack correlacionado por seq,
//! timeout honesto (§4.1/§6), servidor de referência e negociação CAPS (§4.5).

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::registry::RemoteAddr;
use vbl_fxp::schema::{caps, op, AckAct, Body, DeviceDesc};
use vbl_fxp::transport::{serve_tcp, serve_unix, wait_ready_unix, TransportError};
use vbl_fxp::{Message, WireValue as WV};

const DEADLINE: Duration = Duration::from_secs(2);
const SHORT_DEADLINE: Duration = Duration::from_millis(120);

fn tmpsocket(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "vbl-fxp-{}-{}-{}.sock",
        name,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Echo canônico: READ → READ_OK (com canônico + marca sintética);
/// ACT → ACT_ACK Entregue; HEARTBEAT → HEARTBEAT_ACK ok.
fn echo(msg: Message) -> Option<Message> {
    Some(match msg.opcode {
        vbl_fxp::schema::op::READ => Message::read_ok(86.5, "cpu_temp", false, msg.seq),
        vbl_fxp::schema::op::ACT => Message::act_ack(AckAct::Delivered, false, msg.seq),
        vbl_fxp::schema::op::HEARTBEAT => Message::heartbeat_ack(true, msg.seq),
        _ => Message::bye(msg.seq),
    })
}

#[test]
fn unix_roundtrip_with_correlated_ack_and_seq() {
    let path = tmpsocket("echo");
    let _srv = serve_unix(&path, echo).expect("subir servidor");
    assert!(wait_ready_unix(&path, DEADLINE));

    let mut c = vbl_fxp::transport::Connection::unix(&path, DEADLINE).unwrap();

    let r = c
        .request(&Message::read("cpu_temp", 7, true), DEADLINE)
        .unwrap();
    assert_eq!(r.seq, 7);
    let Body::ReadOk { value, canonical } = r.body else {
        panic!("resposta errada")
    };
    assert_eq!((value, canonical.as_str()), (86.5, "cpu_temp"));
    assert_eq!(
        r.flags & vbl_fxp::schema::flag::SYNTHETIC,
        0,
        "leitura real no fio"
    );

    let r = c
        .request(&Message::act("Fan", WV::Num(200.0), 8, true), DEADLINE)
        .unwrap();
    assert_eq!(
        r.body,
        Body::ActAck {
            status: AckAct::Delivered
        }
    );

    // Encerramento limpo (BYE sem ack).
    c.enviar(&Message::bye(0)).unwrap();
}

#[test]
fn remote_tcp_speaks_same_frame_v1() {
    let (srv, port) = serve_tcp(echo).expect("subir servidor tcp");
    let mut c = vbl_fxp::transport::Connection::tcp("127.0.0.1", port, DEADLINE).unwrap();
    let r = c
        .request(&Message::read("cpu_power", 11, true), DEADLINE)
        .unwrap();
    assert_eq!(r.seq, 11);
    srv.parar();
}

#[test]
fn honest_timeout_when_actor_is_mute() {
    let path = tmpsocket("mudo");
    // Servidor que não responde (ator não respondendo — BDD Caso 3).
    let _srv = serve_unix(&path, |_msg| None).expect("subir servidor");
    assert!(wait_ready_unix(&path, DEADLINE));
    let mut c = vbl_fxp::transport::Connection::unix(&path, DEADLINE).unwrap();
    let start = std::time::Instant::now();
    let err = c
        .request(&Message::heartbeat("Fan", 3), SHORT_DEADLINE)
        .unwrap_err();
    assert_eq!(err, TransportError::Timeout);
    assert!(
        start.elapsed() >= SHORT_DEADLINE,
        "timeout não pode retornar antes do prazo"
    );
}

#[test]
fn desynced_seq_and_error_never_swapped_ack() {
    let path = tmpsocket("seq");
    let _srv = serve_unix(&path, |msg| {
        Some(Message::read_ok(1.0, "x", false, msg.seq + 100))
    })
    .expect("subir servidor");
    assert!(wait_ready_unix(&path, DEADLINE));
    let mut c = vbl_fxp::transport::Connection::unix(&path, DEADLINE).unwrap();
    assert!(matches!(
        c.request(&Message::read("x", 1, true), DEADLINE),
        Err(TransportError::Broken(_))
    ));
}

#[test]
fn hello_publishes_peer_registry() {
    let path = tmpsocket("hello");
    let registry = vec![
        DeviceDesc::Sensor {
            name: "cpu_temp".into(),
            min: Some(0.0),
            max: Some(120.0),
            quantity: "temperature".into(),
            unit: "°C".into(),
            precision_pct: 2.0,
        },
        DeviceDesc::Actor {
            name: "CpuPowerCap".into(),
            min: Some(10.0),
            max: Some(250.0),
            safety: Some(200.0),
        },
    ];
    let _srv = serve_unix(&path, move |msg| {
        matches!(msg.opcode, vbl_fxp::schema::op::HELLO)
            .then(|| Message::hello(registry.clone(), msg.seq))
    })
    .expect("subir servidor");
    assert!(wait_ready_unix(&path, DEADLINE));

    let mut c = vbl_fxp::transport::Connection::unix(&path, DEADLINE).unwrap();
    let r = c
        .request(
            &Message::hello(
                vec![DeviceDesc::Actor {
                    name: "Cliente".into(),
                    min: None,
                    max: None,
                    safety: None,
                }],
                5,
            ),
            DEADLINE,
        )
        .unwrap();
    let Body::Hello { devices } = r.body else {
        panic!()
    };
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].name(), "cpu_temp");
}

#[test]
fn connection_to_nonexistent_server_fails_without_panic() {
    let path = tmpsocket("fantasma");
    assert!(matches!(
        vbl_fxp::transport::Connection::unix(&path, DEADLINE),
        Err(TransportError::ConnectionFailed(_))
    ));
    // RemoteAddr descreve os dois esquemas (usado pelo Endpoint::Remote).
    let _ = RemoteAddr::Unix(path);
}

// ══════════════════════════════════════════════════════════════════════════
// v1.1 — Negociação CAPS (docs/FXP-SCHEMA-v1.md §4.5)
// ══════════════════════════════════════════════════════════════════════════

/// Peer v1.1 de teste: concede apenas BATCH|TIMESTAMP (interseção honesta).
fn caps_peer(msg: Message) -> Option<Message> {
    match msg.opcode {
        op::CAPS => {
            let Body::Caps { capabilities } = msg.body else {
                return None;
            };
            Some(Message::caps_ok(
                capabilities & (caps::BATCH | caps::TIMESTAMP),
                msg.seq,
            ))
        }
        _ => None,
    }
}

#[test]
fn negociacao_caps_confirma_a_intersecao() {
    let (srv, port) = serve_tcp(caps_peer).expect("subir servidor");
    let mut c = vbl_fxp::transport::Connection::tcp("127.0.0.1", port, DEADLINE).unwrap();
    let concedidas = c
        .negotiate(caps::LZ4 | caps::BATCH | caps::TIMESTAMP, DEADLINE)
        .expect("negociação");
    assert_eq!(
        concedidas,
        caps::BATCH | caps::TIMESTAMP,
        "peer não anuncia LZ4"
    );
    assert_eq!(c.negotiated_caps(), caps::BATCH | caps::TIMESTAMP);
    srv.parar();
}

#[test]
fn negociacao_sem_capacidade_alguma_e_no_op() {
    let (srv, port) = serve_tcp(caps_peer).expect("subir servidor");
    let mut c = vbl_fxp::transport::Connection::tcp("127.0.0.1", port, DEADLINE).unwrap();
    // Nada pedido ⇒ nada no fio, estado zerado.
    assert_eq!(c.negotiate(0, DEADLINE).unwrap(), 0);
    assert_eq!(c.negotiated_caps(), 0);
    srv.parar();
}

#[test]
fn negociacao_sem_resposta_do_peer_da_timeout() {
    let (srv, port) = serve_tcp(|_msg| None).expect("subir servidor");
    let mut c = vbl_fxp::transport::Connection::tcp("127.0.0.1", port, DEADLINE).unwrap();
    let r = c.negotiate(caps::BATCH, SHORT_DEADLINE);
    assert!(
        matches!(r, Err(TransportError::Timeout)),
        "esperado Timeout, veio {r:?}"
    );
    srv.parar();
}

#[test]
fn peer_v1_0_diante_de_caps_falha_fechado() {
    // Servidor v1.0 simulado com TCP cru: lê um frame, vê opcode desconhecido
    // (0x06 = CAPS não existe no v1) e fecha a conexão — nunca interpreta.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf); // lê o CAPS (e ignora o conteúdo)
            drop(s); // v1.0 real rejeitaria o opcode e cairia
        }
    });
    let mut c = vbl_fxp::transport::Connection::tcp("127.0.0.1", port, DEADLINE).unwrap();
    let r = c.negotiate(caps::BATCH, DEADLINE);
    assert!(
        matches!(r, Err(TransportError::Broken(_))),
        "peer v1.0 fechando a conexão ⇒ Broken (fail closed), veio {r:?}"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// v1.1 §4.6 — caminhos de erro do handshake AUTH (cliente × servidor cru).
// ══════════════════════════════════════════════════════════════════════════

/// Servidor TCP cru que devolve o frame dado como primeira mensagem.
fn servidor_cru_primeira(
    msg_bytes: Vec<u8>,
) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            use std::io::Write;
            let _ = stream.write_all(&msg_bytes);
            let _ = stream.flush();
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    });
    (addr, handle)
}

#[test]
fn authenticate_primeira_mensagem_nao_e_challenge_falha() {
    use vbl_fxp::schema::{encode_to_vec, Message};
    // READ_OK forjado como primeira mensagem.
    let bytes = encode_to_vec(&Message::read_ok(1.0, "cpu_temp", true, 7)).unwrap();
    let (addr, handle) = servidor_cru_primeira(bytes);
    let mut c =
        vbl_fxp::transport::Connection::tcp("127.0.0.1", addr.port(), Duration::from_secs(1))
            .expect("conectar");
    let err = c
        .authenticate(b"chave", Duration::from_secs(1))
        .unwrap_err();
    assert!(matches!(err, vbl_fxp::TransportError::Broken(m) if m.contains("AUTH_CHALLENGE")));
    handle.join().ok();
}

#[test]
fn authenticate_scheme_desconhecido_falha() {
    use vbl_fxp::schema::{encode_to_vec, Message};
    let mut bytes = encode_to_vec(&Message::auth_challenge(1, [0u8; 32], 1)).unwrap();
    // O encode REJEITA scheme 9 (contrato); o servidor cru forja os bytes.
    bytes[4 + 12] = 9;
    bytes[4 + 13] = 0;
    let (addr, handle) = servidor_cru_primeira(bytes);
    let mut c =
        vbl_fxp::transport::Connection::tcp("127.0.0.1", addr.port(), Duration::from_secs(1))
            .expect("conectar");
    // Rejeição TIPADA do schema (não confunde com quebra de transporte).
    let err = c
        .authenticate(b"chave", Duration::from_secs(1))
        .unwrap_err();
    assert!(matches!(
        err,
        vbl_fxp::TransportError::Schema(vbl_fxp::SchemaError::UnknownAuthScheme { received: 9 })
    ));
    handle.join().ok();
}

#[test]
fn authenticate_resposta_sem_auth_ok_falha() {
    // Challenge válido; na resposta do cliente, o servidor responde HELLO
    // (não AUTH_OK) ⇒ recusa.
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    use vbl_fxp::schema::{encode_to_vec, Message};
    let challenge = encode_to_vec(&Message::auth_challenge(1, [7u8; 32], 1)).unwrap();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(&challenge);
            let _ = stream.flush();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf); // consome AUTH_RESPONSE do cliente
            let hello = Message::hello(Vec::new(), 1); // eco do seq do pedido
            let _ = stream.write_all(&encode_to_vec(&hello).unwrap());
            let _ = stream.flush();
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    });
    let mut c =
        vbl_fxp::transport::Connection::tcp("127.0.0.1", addr.port(), Duration::from_secs(1))
            .expect("conectar");
    let err = c
        .authenticate(b"chave", Duration::from_secs(1))
        .unwrap_err();
    assert!(
        matches!(err, vbl_fxp::TransportError::Broken(ref m) if m.contains("recusado")),
        "erro inesperado: {err:?}"
    );
    handle.join().ok();
}

// ══════════════════════════════════════════════════════════════════════════
// Robustez do receive: frame partido em duas escritas (buffering), servidor
// que some no meio do frame e opcode genérico pré-autenticação.
// ══════════════════════════════════════════════════════════════════════════

/// Servidor TCP cru que escreve o frame em DOIS pedaços (50 ms entre eles).
fn servidor_frame_partido(bytes: Vec<u8>) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        let meio = bytes.len() / 2;
        use std::io::Write;
        s.write_all(&bytes[..meio]).unwrap();
        s.flush().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        s.write_all(&bytes[meio..]).unwrap();
        s.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300)); // dá tempo de o cliente ler
    });
    (addr, handle)
}

#[test]
fn receive_monta_frame_que_chega_partido() {
    use vbl_fxp::schema::{decode, encode_to_vec, Message};
    let challenge = Message::auth_challenge(1, [3u8; 32], 1);
    let bytes = encode_to_vec(&challenge).unwrap();
    let (addr, handle) = servidor_frame_partido(bytes);
    let mut c =
        vbl_fxp::transport::Connection::tcp("127.0.0.1", addr.port(), Duration::from_secs(2))
            .expect("conectar");
    let msg = c
        .receive(Duration::from_secs(2))
        .expect("challenge remontado de dois pedaços");
    let (back, _) = decode(&encode_to_vec(&msg).unwrap()).unwrap();
    assert_eq!(back, challenge);
    handle.join().ok();
}

#[test]
fn servidor_que_some_no_meio_do_frame_da_erro_honesto() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        use std::io::Write;
        // METADE de um frame e fecha na sequência.
        s.write_all(&[0x40, 0x00, 0x00, 0x00, b'F', b'X', b'P'])
            .unwrap();
        s.flush().unwrap();
        drop(s);
        drop(listener);
    });
    let mut c =
        vbl_fxp::transport::Connection::tcp("127.0.0.1", addr.port(), Duration::from_secs(2))
            .expect("conectar");
    let err = c.receive(Duration::from_secs(2)).unwrap_err();
    assert!(
        matches!(err, vbl_fxp::TransportError::Broken(ref m) if m.contains("leitura") || m.contains("fluxo")),
        "erro inesperado: {err:?}"
    );
    handle.join().ok();
}

#[test]
fn conexoes_recusadas_falham_honesto() {
    use std::time::Duration;
    // TCP: host não resolvível e porta fechada ⇒ ConnectionFailed tipado.
    let dns = vbl_fxp::transport::Connection::tcp(
        "host_invalido_sem_ponto",
        1,
        Duration::from_millis(200),
    )
    .unwrap_err();
    assert!(
        matches!(dns, vbl_fxp::TransportError::ConnectionFailed(_)),
        "{dns:?}"
    );
    let recusada = vbl_fxp::transport::Connection::tcp("127.0.0.1", 1, Duration::from_millis(200))
        .unwrap_err();
    assert!(
        matches!(recusada, vbl_fxp::TransportError::ConnectionFailed(_)),
        "{recusada:?}"
    );
    // Unix: caminho inexistente.
    let unix = vbl_fxp::transport::Connection::unix(
        std::path::Path::new("/tmp/vbl-não-existe-xyz.sock"),
        Duration::from_millis(200),
    )
    .unwrap_err();
    assert!(
        matches!(unix, vbl_fxp::TransportError::ConnectionFailed(_)),
        "{unix:?}"
    );
}

#[test]
fn receive_sem_resposta_respeita_o_timeout() {
    // Servidor que aceita e NÃO responde: o cliente estoura o prazo com
    // Timeout (não fica preso no WouldBlock para sempre).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let (_s, _) = listener.accept().expect("accept");
        std::thread::sleep(Duration::from_millis(600)); // mudo
    });
    let mut c =
        vbl_fxp::transport::Connection::tcp("127.0.0.1", addr.port(), Duration::from_secs(1))
            .expect("conectar");
    let err = c.receive(Duration::from_millis(200)).unwrap_err();
    assert!(matches!(err, vbl_fxp::TransportError::Timeout), "{err:?}");
    handle.join().ok();
}

#[test]
fn frame_com_tamanho_zero_e_recusado() {
    // Prefixo de length 0: peek_frame_len recusa sem travar o cliente.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        use std::io::Write;
        s.write_all(&[0, 0, 0, 0]).unwrap();
        s.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    });
    let mut c =
        vbl_fxp::transport::Connection::tcp("127.0.0.1", addr.port(), Duration::from_secs(1))
            .expect("conectar");
    let err = c.receive(Duration::from_millis(500)).unwrap_err();
    assert!(
        matches!(err, vbl_fxp::TransportError::Schema(_)),
        "erro inesperado: {err:?}"
    );
    handle.join().ok();
}

#[test]
fn receive_monta_dois_frames_que_chegam_juntos_e_partidos() {
    // Um único write com DOIS frames completos: o cliente deve devolver o
    // primeiro e manter o segundo no buffer (receive encadeado).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let f1 = vbl_fxp::schema::encode_to_vec(&Message::read("temp_a", 1, true)).unwrap();
    let f2 = vbl_fxp::schema::encode_to_vec(&Message::read("temp_b", 2, true)).unwrap();
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        use std::io::Write;
        let mut dois = f1.clone();
        dois.extend_from_slice(&f2);
        s.write_all(&dois).unwrap();
        s.flush().unwrap();
        std::thread::sleep(Duration::from_millis(400));
    });
    let mut c =
        vbl_fxp::transport::Connection::tcp("127.0.0.1", addr.port(), Duration::from_secs(1))
            .expect("conectar");
    let m1 = c.receive(Duration::from_millis(500)).unwrap();
    let m2 = c.receive(Duration::from_millis(500)).unwrap();
    assert_eq!(m1.name, "temp_a");
    assert_eq!(m2.name, "temp_b");
    handle.join().ok();
}

#[test]
fn compressao_que_infla_nao_viaja() {
    // Mensagem pequena com LZ4 negociado: comprimir cresceria, então o
    // codec manda o payload plano (§4.8) — ida e volta segue intacta.
    let mut f = Vec::new();
    let m = Message::heartbeat("x", 7);
    vbl_fxp::schema::encode_with_compression(&m, &mut f).expect("encode comprimido");
    let (back, _) = vbl_fxp::schema::decode(&f).expect("decode do frame comprimido");
    assert_eq!(back.name, "x");
    assert_eq!(back.body, m.body);
}
