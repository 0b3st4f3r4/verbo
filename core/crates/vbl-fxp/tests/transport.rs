//! Transporte Unix/TCP do schema v1: roundtrip, ack correlacionado por seq,
//! timeout honesto (§4.1/§6) e servidor de referência.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::registry::RemoteAddr;
use vbl_fxp::schema::{AckAct, Corpo, DeviceDesc};
use vbl_fxp::transport::{esperar_pronto_unix, servir_tcp, servir_unix, ErroTransporte};
use vbl_fxp::{Mensagem, WireValue as WV};

const PRAZO: Duration = Duration::from_secs(2);
const PRAZO_CURTO: Duration = Duration::from_millis(120);

fn tmpsocket(nome: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "vbl-fxp-{}-{}-{}.sock",
        nome,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Echo canônico: READ → READ_OK (com canônico + marca sintética);
/// ACT → ACT_ACK Entregue; HEARTBEAT → HEARTBEAT_ACK ok.
fn echo(msg: Mensagem) -> Option<Mensagem> {
    Some(match msg.opcode {
        vbl_fxp::schema::op::READ => {
            Mensagem::read_ok(86.5, "cpu_temp", false, msg.seq)
        }
        vbl_fxp::schema::op::ACT => Mensagem::act_ack(AckAct::Entregue, false, msg.seq),
        vbl_fxp::schema::op::HEARTBEAT => Mensagem::heartbeat_ack(true, msg.seq),
        _ => Mensagem::bye(msg.seq),
    })
}

#[test]
fn unix_roundtrip_com_ack_e_seq_correlacionado() {
    let path = tmpsocket("echo");
    let _srv = servir_unix(&path, echo).expect("subir servidor");
    assert!(esperar_pronto_unix(&path, PRAZO));

    let mut c = vbl_fxp::transport::Conexao::unix(&path, PRAZO).unwrap();

    let r = c.pedir(&Mensagem::read("cpu_temp", 7, true), PRAZO).unwrap();
    assert_eq!(r.seq, 7);
    let Corpo::ReadOk { valor, canonical } = r.corpo else { panic!("resposta errada") };
    assert_eq!((valor, canonical.as_str()), (86.5, "cpu_temp"));
    assert_eq!(r.flags & vbl_fxp::schema::flag::SINTETICO, 0, "leitura real no fio");

    let r = c
        .pedir(&Mensagem::act("Ventoinha", WV::Num(200.0), 8, true), PRAZO)
        .unwrap();
    assert_eq!(r.corpo, Corpo::ActAck { status: AckAct::Entregue });

    // Encerramento limpo (BYE sem ack).
    c.enviar(&Mensagem::bye(0)).unwrap();
}

#[test]
fn tcp_remoto_fala_o_mesmo_frame_v1() {
    let (srv, port) = servir_tcp(echo).expect("subir servidor tcp");
    let mut c = vbl_fxp::transport::Conexao::tcp("127.0.0.1", port, PRAZO).unwrap();
    let r = c.pedir(&Mensagem::read("cpu_power", 11, true), PRAZO).unwrap();
    assert_eq!(r.seq, 11);
    srv.parar();
}

#[test]
fn timeout_honesto_quando_o_ator_e_mudo() {
    let path = tmpsocket("mudo");
    // Servidor que não responde (ator não respondendo — BDD Caso 3).
    let _srv = servir_unix(&path, |_msg| None).expect("subir servidor");
    assert!(esperar_pronto_unix(&path, PRAZO));
    let mut c = vbl_fxp::transport::Conexao::unix(&path, PRAZO).unwrap();
    let inicio = std::time::Instant::now();
    let err = c
        .pedir(&Mensagem::heartbeat("Ventoinha", 3), PRAZO_CURTO)
        .unwrap_err();
    assert_eq!(err, ErroTransporte::Timeout);
    assert!(inicio.elapsed() >= PRAZO_CURTO, "timeout não pode retornar antes do prazo");
}

#[test]
fn seq_dessincronizado_e_erro_nunca_ack_trocado() {
    let path = tmpsocket("seq");
    let _srv = servir_unix(&path, |msg| Some(Mensagem::read_ok(1.0, "x", false, msg.seq + 100)))
        .expect("subir servidor");
    assert!(esperar_pronto_unix(&path, PRAZO));
    let mut c = vbl_fxp::transport::Conexao::unix(&path, PRAZO).unwrap();
    assert!(matches!(
        c.pedir(&Mensagem::read("x", 1, true), PRAZO),
        Err(ErroTransporte::Quebrada(_))
    ));
}

#[test]
fn hello_publica_o_registro_do_peer() {
    let path = tmpsocket("hello");
    let registro = vec![
        DeviceDesc::Sensor {
            name: "cpu_temp".into(),
            min: Some(0.0),
            max: Some(120.0),
            grandeza: "temperatura".into(),
            unidade: "°C".into(),
            precisao_pct: 2.0,
        },
        DeviceDesc::Ator {
            name: "CpuPowerCap".into(),
            min: Some(10.0),
            max: Some(250.0),
            safety: Some(200.0),
        },
    ];
    let _srv = servir_unix(&path, move |msg| {
        matches!(msg.opcode, vbl_fxp::schema::op::HELLO)
            .then(|| Mensagem::hello(registro.clone(), msg.seq))
    })
    .expect("subir servidor");
    assert!(esperar_pronto_unix(&path, PRAZO));

    let mut c = vbl_fxp::transport::Conexao::unix(&path, PRAZO).unwrap();
    let r = c
        .pedir(&Mensagem::hello(vec![DeviceDesc::Ator {
            name: "Cliente".into(),
            min: None,
            max: None,
            safety: None,
        }], 5), PRAZO)
        .unwrap();
    let Corpo::Hello { devices } = r.corpo else { panic!() };
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].name(), "cpu_temp");
}

#[test]
fn conexao_a_servidor_inexistente_falha_sem_panico() {
    let path = tmpsocket("fantasma");
    assert!(matches!(
        vbl_fxp::transport::Conexao::unix(&path, PRAZO),
        Err(ErroTransporte::ConexaoFalhou(_))
    ));
    // RemoteAddr descreve os dois esquemas (usado pelo Endpoint::Remote).
    let _ = RemoteAddr::Unix(path);
}
