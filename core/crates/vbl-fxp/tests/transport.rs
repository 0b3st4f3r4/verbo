//! Transporte Unix/TCP do schema v1: roundtrip, ack correlacionado por seq,
//! timeout honesto (§4.1/§6) e servidor de referência.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::registry::RemoteAddr;
use vbl_fxp::schema::{AckAct, Body, DeviceDesc};
use vbl_fxp::transport::{wait_ready_unix, serve_tcp, serve_unix, TransportError};
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
        vbl_fxp::schema::op::READ => {
            Message::read_ok(86.5, "cpu_temp", false, msg.seq)
        }
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

    let r = c.request(&Message::read("cpu_temp", 7, true), DEADLINE).unwrap();
    assert_eq!(r.seq, 7);
    let Body::ReadOk { value, canonical } = r.body else { panic!("resposta errada") };
    assert_eq!((value, canonical.as_str()), (86.5, "cpu_temp"));
    assert_eq!(r.flags & vbl_fxp::schema::flag::SYNTHETIC, 0, "leitura real no fio");

    let r = c
        .request(&Message::act("Ventoinha", WV::Num(200.0), 8, true), DEADLINE)
        .unwrap();
    assert_eq!(r.body, Body::ActAck { status: AckAct::Delivered });

    // Encerramento limpo (BYE sem ack).
    c.enviar(&Message::bye(0)).unwrap();
}

#[test]
fn remote_tcp_speaks_same_frame_v1() {
    let (srv, port) = serve_tcp(echo).expect("subir servidor tcp");
    let mut c = vbl_fxp::transport::Connection::tcp("127.0.0.1", port, DEADLINE).unwrap();
    let r = c.request(&Message::read("cpu_power", 11, true), DEADLINE).unwrap();
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
        .request(&Message::heartbeat("Ventoinha", 3), SHORT_DEADLINE)
        .unwrap_err();
    assert_eq!(err, TransportError::Timeout);
    assert!(start.elapsed() >= SHORT_DEADLINE, "timeout não pode retornar antes do prazo");
}

#[test]
fn desynced_seq_and_error_never_swapped_ack() {
    let path = tmpsocket("seq");
    let _srv = serve_unix(&path, |msg| Some(Message::read_ok(1.0, "x", false, msg.seq + 100)))
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
            quantity: "temperatura".into(),
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
        .request(&Message::hello(vec![DeviceDesc::Actor {
            name: "Cliente".into(),
            min: None,
            max: None,
            safety: None,
        }], 5), DEADLINE)
        .unwrap();
    let Body::Hello { devices } = r.body else { panic!() };
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
