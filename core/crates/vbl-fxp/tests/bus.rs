//! Integração do `FxpBus`: modos real/simulado/híbrido, honestidade de dados
//! (FORMAL §4.7), cache, fallback do registro (§4.3), fila prioritária com
//! re-entrega/expiração, transporte remoto e o cenário capitular da Etapa 3:
//! subversão térmica E2E com drivers reais em fixtures.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::bus::{BusConfig, FxpBus};
use vbl_fxp::queue::PRIORITY_SUBVERT;
use vbl_fxp::registry::{DeviceRegistry, FxpConfig, OperationMode};
use vbl_fxp::schema::AckAct;
use vbl_fxp::transport::{serve_unix, wait_ready_unix};
use vbl_runtime::fxp::{ActOutcome, Fxp, Limit, SensorFailure, Value};
use vbl_runtime::ledger::ChainLedger;
use vbl_runtime::{load, Engine};

const DEADLINE: Duration = Duration::from_secs(2);

fn tmpdir(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vbl-bus-{}-{}-{}",
        name,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("criar tmpdir");
    dir
}

/// thermal_zone sintética: escreve `temp` em mili°C.
fn fixture_thermal(dir: &Path, millic: i64) {
    fs::write(dir.join("temp"), millic.to_string()).unwrap();
}

fn bus_simulated() -> (FxpBus, ChainLedger) {
    let cfg = BusConfig {
        mode: OperationMode::Simulated,
        ..Default::default()
    };
    (
        FxpBus::build(
            DeviceRegistry::minimum(),
            cfg,
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
    )
}

/// Bus híbrido com cpu_temp (thermal), Fan (pwm) e CpuPowerCap (cap)
/// em rotas REAIS de fixture. Recebe os CAMINHOS DOS ARQUIVOS de pwm/cap.
fn bus_hybrid_fixture(tz_dir: &Path, pwm_file: &Path, cap_file: &Path) -> (FxpBus, ChainLedger) {
    let cfg_text = format!(
        "mode = hibrido\n\
         cache_ttl_ms = 0\n\
         cpu_temp.mode = real\n\
         cpu_temp.endpoint = thermal_zone:{}\n\
         Fan.mode = real\n\
         Fan.endpoint = hwmon_pwm:{}\n\
         CpuPowerCap.mode = real\n\
         CpuPowerCap.endpoint = rapl_constraint:{}\n",
        tz_dir.display(),
        pwm_file.display(),
        cap_file.display()
    );
    let cfg = FxpConfig::parse(&cfg_text).unwrap();
    let mut registry = DeviceRegistry::minimum();
    cfg.apply(&mut registry).unwrap();
    let bus_cfg = BusConfig {
        mode: OperationMode::Hybrid,
        cache_ttl: Duration::ZERO, // determinismo nos testes
        ..Default::default()
    };
    (
        FxpBus::build(registry, bus_cfg, vbl_runtime::FxpSimulator::new()),
        ChainLedger::new(),
    )
}

// ---------------------------------------------------------------------------
// Modo simulado: paridade com a Etapa 2
// ---------------------------------------------------------------------------

#[test]
fn simulated_mode_has_parity_with_stage_2() {
    let (mut bus, mut ledger) = bus_simulated();

    // Leitura: valores plausíveis do simulador.
    assert_eq!(bus.read_sensor("cpu_temp", &mut ledger).unwrap(), 55.0);
    bus.sim_mut().set_sensor("cpu_temp", 86.5);
    assert_eq!(bus.read_sensor("cpu_temp", &mut ledger).unwrap(), 86.5);

    // Limites inclusivos: 200 passa; 200.1 e 201 violam o safety_limit.
    assert_eq!(
        bus.act("CpuPowerCap", Value::Num(200.0), &mut ledger),
        ActOutcome::Delivered
    );
    assert!(matches!(
        bus.act("CpuPowerCap", Value::Num(200.1), &mut ledger),
        ActOutcome::Rejected {
            limit: Limit::SafetyLimit,
            limit_value: 200.0
        }
    ));

    // cpu_power para partilha P/N vem do simulador.
    assert_eq!(bus.cpu_power(), 150.0);

    // Sensor ausente: nunca 0.0 (§4.7).
    assert_eq!(
        bus.read_sensor("nao_existe", &mut ledger),
        Err(SensorFailure::NotRegistered)
    );
}

// ---------------------------------------------------------------------------
// Híbrido: rotas reais em fixtures
// ---------------------------------------------------------------------------

#[test]
fn hybrid_reads_real_driver_and_converts_unit() {
    let tz = tmpdir("tz");
    fixture_thermal(&tz, 86_500);
    let pwm = tmpdir("pwm").join("pwm1");
    let cap = tmpdir("cap").join("constraint_0_power_limit_uw");
    fs::write(&pwm, "0").unwrap();
    fs::write(&cap, "250000000").unwrap();

    let (mut bus, mut ledger) = bus_hybrid_fixture(&tz, &pwm, &cap);

    // mili°C → °C exato.
    assert_eq!(bus.read_sensor("cpu_temp", &mut ledger).unwrap(), 86.5);
    // attention continua simulada (não configurada como real).
    assert_eq!(bus.read_sensor("attention", &mut ledger).unwrap(), 100.0);
}

#[test]
fn real_mode_without_route_inaccessible_never_fabricated() {
    // Global REAL: attention (simulada no registro) fica inacessível — o bus
    // não fabrica leitura sintética em modo real (§4.7).
    let cfg = BusConfig {
        mode: OperationMode::Real,
        ..Default::default()
    };
    let (mut bus, mut ledger) = (
        FxpBus::build(
            DeviceRegistry::minimum(),
            cfg,
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
    );
    assert_eq!(
        bus.read_sensor("attention", &mut ledger),
        Err(SensorFailure::Inaccessible)
    );
    assert!(ledger
        .search("ALERT", &[])
        .iter()
        .any(|e| e.msg.contains("attention")));
    // A rota documenta o motivo (honestidade observável).
    assert!(bus
        .route_of("attention")
        .unwrap()
        .description()
        .contains("inacessível"));
}

#[test]
fn cache_ttl_saves_real_reads() {
    let tz = tmpdir("tz-cache");
    fixture_thermal(&tz, 50_000);
    let cfg_text = format!(
        "mode = hibrido\ncache_ttl_ms = 60000\ncpu_temp.mode = real\ncpu_temp.endpoint = thermal_zone:{}\n",
        tz.display()
    );
    let cfg = FxpConfig::parse(&cfg_text).unwrap();
    let mut registry = DeviceRegistry::minimum();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig {
            mode: OperationMode::Hybrid,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    assert_eq!(bus.read_sensor("cpu_temp", &mut ledger).unwrap(), 50.0);
    // Mundo muda; cache (TTL 60 s) ainda vale → leitura envelhecida.
    fixture_thermal(&tz, 90_000);
    assert_eq!(bus.read_sensor("cpu_temp", &mut ledger).unwrap(), 50.0);
}

// ---------------------------------------------------------------------------
// Cenário capitular: subversão térmica E2E com drivers reais
// ---------------------------------------------------------------------------

#[test]
fn thermal_subversion_e2e_with_real_drivers() {
    // BDD Caso 2 na Etapa 3: `when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50)`
    // sobre o BUS híbrido com cpu_temp e CpuPowerCap REAIS (fixtures sysfs).
    let tz = tmpdir("tz-subvert");
    fixture_thermal(&tz, 86_500); // 86.5°C > 85°C
    let cap = tmpdir("cap-subvert").join("constraint_0_power_limit_uw");
    fs::write(&cap, "250000000").unwrap();
    let pwm = tmpdir("pwm-subvert").join("pwm1");
    fs::write(&pwm, "0").unwrap();

    let (bus, _) = bus_hybrid_fixture(&tz, &pwm, &cap);
    let mut engine = Engine::new(bus, 1.0, tmpdir("persist-subvert"));

    let source = r#"
nonequilibrium SpeculativeTrading {
    value: "lucro_arbitragem_alta_frequencia",
    horizon: 7s,
    source_path: "cpu_temp",
    maintenance_deadline: 2s,
    exchange_mode: "extraction"
}

review SpeculativeTrading {
    when cpu_temp > 85°C -> subvert,
                            act(CpuPowerCap, 50)
}
"#;
    let (program, diags) = vbl_lang::parse(source);
    assert!(
        diags.errors().next().is_none(),
        "programa deve parsear: {diags:?}"
    );
    let _ = load(&mut engine, &program);

    engine.tick();

    // 1) subvert dissolve no mesmo tick (§4.5).
    assert!(
        engine.form("SpeculativeTrading").is_none(),
        "forma subvertida deve dissolver no tick"
    );
    // 2) a act pós-subvert chegou ao DRIVER REAL em µW (50 W).
    let written = fs::read_to_string(&cap).unwrap();
    assert_eq!(
        written, "50000000",
        "CpuPowerCap real deve receber 50 W em µW"
    );
    // 3) trilha no Caderno: ATUACAO sucesso + SUBVERSAO.
    assert!(engine
        .ledger
        .search("ACTUATION", &[])
        .iter()
        .any(|e| e.msg.contains("CpuPowerCap") && e.msg.contains("sucesso")));
    assert!(!engine.ledger.search("SUBVERSION", &[]).is_empty());
    // 4) valor poético canônico aplicado (§4.5).
    assert!(!engine.ledger.search("dissolve_subvert", &[]).is_empty());
}

// ---------------------------------------------------------------------------
// Fallback do registro (§4.3) com rotas reais
// ---------------------------------------------------------------------------

#[test]
fn real_fallback_alternate_actor_full_trail() {
    let dir = tmpdir("fallback");
    let prim = dir.join("pwm1");
    let alt = dir.join("pwm2");
    fs::write(&prim, "0").unwrap();
    fs::write(&alt, "0").unwrap();

    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\n\
         Fan.mode = real\nFan.endpoint = hwmon_pwm:{}\n\
         ReserveFan.mode = real\nReserveFan.endpoint = hwmon_pwm:{}\n\
         ReserveFan.min = 0\nReserveFan.max = 255\n\
         fallback.Fan = ReserveFan\n",
        prim.display(),
        alt.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig {
            mode: OperationMode::Hybrid,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    // Primário OK: entrega direta no pwm1.
    assert_eq!(
        bus.act("Fan", Value::Num(200.0), &mut ledger),
        ActOutcome::Delivered
    );
    assert_eq!(fs::read_to_string(&prim).unwrap(), "200");

    // Primário morre (endpoint removido) → fallback do registro → pwm2.
    fs::remove_file(&prim).unwrap();
    let outcome = bus.act("Fan", Value::Num(180.0), &mut ledger);
    assert!(matches!(outcome,
        ActOutcome::FallbackExecuted { ref alternativo } if alternativo == "ReserveFan"));
    assert_eq!(fs::read_to_string(&alt).unwrap(), "180");

    // Trilha completa: indisponível + fallback executado.
    assert!(!ledger.search("actor_unavailable", &[]).is_empty());
    assert!(!ledger.search("fallback_executed", &[]).is_empty());
}

// ---------------------------------------------------------------------------
// Fila prioritária: re-entrega e expiração no relógio virtual
// ---------------------------------------------------------------------------

#[test]
fn queue_redelivers_and_expires_with_audit() {
    let dir = tmpdir("fila");
    let prim = dir.join("pwm1");
    fs::write(&prim, "0").unwrap();

    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\n\
         Fan.mode = real\nFan.endpoint = hwmon_pwm:{}\n",
        prim.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig {
            mode: OperationMode::Hybrid,
            queue_timeout_ticks: 2,
            retries: 0, // determinismo: uma tentativa por tick
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    // Ator morto: esgota → comando entra na fila com prioridade de subvert.
    fs::remove_file(&prim).unwrap();
    let outcome = bus.act_with_priority("Fan", Value::Num(120.0), PRIORITY_SUBVERT, &mut ledger);
    assert_eq!(outcome, ActOutcome::FallbackExhausted);
    assert_eq!(bus.pending_queue(), 1);

    // Tick 1: endpoint ainda morto → continua pendente (0 < 2, devolvido com 1).
    bus.on_tick(&mut ledger);
    assert_eq!(bus.pending_queue(), 1);

    // Ator volta → tick 2 re-entrega.
    fs::write(&prim, "0").unwrap();
    bus.on_tick(&mut ledger);
    assert_eq!(bus.pending_queue(), 0);
    assert_eq!(fs::read_to_string(&prim).unwrap(), "120");
    assert!(!ledger.search("comando_reentregue", &[]).is_empty());

    // Expiração: com o ator morto de novo, o comando pendente é descartado
    // no tick que atinge queue_timeout_ticks (2), com evento + alerta.
    fs::remove_file(&prim).unwrap();
    let outcome = bus.act("Fan", Value::Num(60.0), &mut ledger);
    assert_eq!(outcome, ActOutcome::FallbackExhausted);
    bus.on_tick(&mut ledger); // ticks_waiting 0→1
    bus.on_tick(&mut ledger); // 1→2
    bus.on_tick(&mut ledger); // 2 >= 2 → expira
    assert_eq!(bus.pending_queue(), 0);
    assert!(!ledger.search("comando_expirado", &[]).is_empty());
    assert!(ledger
        .search("ALERT", &[])
        .iter()
        .any(|e| e.msg.contains("expirou")));
}

// ---------------------------------------------------------------------------
// Alias no Caderno: nome usado + canônico (FORMAL §6)
// ---------------------------------------------------------------------------

#[test]
fn read_by_alias_records_used_and_canonical() {
    let mut registry = DeviceRegistry::minimum();
    registry.set_alias("human_attention", "attention").unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig::default(),
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    assert_eq!(
        bus.read_sensor("human_attention", &mut ledger).unwrap(),
        100.0
    );
    // Evento de mapeamento com o canônico (a LEITURA com o nome usado é do
    // consumidor — no engine — e é coberta no teste E2E).
    assert!(ledger
        .search("INFO", &[])
        .iter()
        .any(|e| e.msg.contains("attention") && e.msg.contains("canônico")));

    // Alias de sensor não serve de ator (kinds são distintos).
    assert_eq!(
        bus.act("human_attention", Value::Num(1.0), &mut ledger),
        ActOutcome::MissingActor
    );
}

// ---------------------------------------------------------------------------
// Rota remota: schema v1 sobre Unix com servidor de referência
// ---------------------------------------------------------------------------

#[test]
fn remote_route_sensor_and_actor_via_schema_v1() {
    let sock = tmpdir("remoto").join("fxpd.sock");
    let _srv = serve_unix(&sock, |msg| match msg.opcode {
        vbl_fxp::schema::op::READ => Some(vbl_fxp::Message::read_ok(
            77.5,
            "solar_panel",
            false,
            msg.seq,
        )),
        vbl_fxp::schema::op::ACT => {
            Some(vbl_fxp::Message::act_ack(AckAct::Delivered, false, msg.seq))
        }
        _ => None,
    })
    .expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\n\
         cache_ttl_ms = 0\n\
         solar_panel.grandeza = luz\nsolar_panel.unidade = W/m2\n\
         solar_panel.mode = real\nsolar_panel.endpoint = unix:{}\n\
         Bomba.mode = real\nBomba.endpoint = unix:{}\nBomba.min = 0\nBomba.max = 100\n",
        sock.display(),
        sock.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig {
            mode: OperationMode::Hybrid,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    assert_eq!(bus.read_sensor("solar_panel", &mut ledger).unwrap(), 77.5);
    assert_eq!(
        bus.act("Bomba", Value::Num(50.0), &mut ledger),
        ActOutcome::Delivered
    );
    // Limite validado LOCALMENTE antes do envio (§4.3).
    assert!(matches!(
        bus.act("Bomba", Value::Num(101.0), &mut ledger),
        ActOutcome::Rejected {
            limit: Limit::Max,
            limit_value: 100.0
        }
    ));
}

#[test]
fn mute_remote_actor_becomes_unavailable_and_queued() {
    let sock = tmpdir("mudo2").join("fxpd.sock");
    let _srv = serve_unix(&sock, |_| None).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));

    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\n\
         Bomba.mode = real\nBomba.endpoint = unix:{}\nBomba.min = 0\nBomba.max = 100\n",
        sock.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig {
            mode: OperationMode::Hybrid,
            act_timeout_local: Duration::from_millis(100),
            queue_timeout_ticks: 1,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    let outcome = bus.act("Bomba", Value::Num(10.0), &mut ledger);
    assert_eq!(outcome, ActOutcome::FallbackExhausted);
    assert!(!ledger.search("actor_unavailable", &[]).is_empty());
    assert_eq!(bus.pending_queue(), 1);

    // tick 1: mudo de novo → re-enfileirado com 1 tick de espera…
    bus.on_tick(&mut ledger);
    assert_eq!(bus.pending_queue(), 1);
    // …tick 2: 1 >= queue_timeout_ticks(1) → expira com auditoria.
    bus.on_tick(&mut ledger);
    assert_eq!(bus.pending_queue(), 0);
    assert!(!ledger.search("comando_expirado", &[]).is_empty());
}

// ---------------------------------------------------------------------------
// Extensões do registro chegam ao backend simulado (fonte única)
// ---------------------------------------------------------------------------

#[test]
fn registry_extension_visible_in_simulator_and_runtime() {
    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(
        "ReserveFan.min = 0\nReserveFan.max = 255\n\
         fallback.Fan = ReserveFan\n",
    )
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let (mut bus, mut ledger) = (
        FxpBus::build(
            registry,
            BusConfig::default(),
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
    );

    // Runtime registry (validação do loader) inclui a extensão…
    assert!(bus.registry().actors.contains_key("ReserveFan"));
    // …e o backend simulado roteia a extensão com os limites do registro.
    assert!(matches!(
        bus.act("ReserveFan", Value::Num(300.0), &mut ledger),
        ActOutcome::Rejected {
            limit: Limit::Max,
            limit_value: 255.0
        }
    ));
    assert_eq!(
        bus.act("ReserveFan", Value::Num(255.0), &mut ledger),
        ActOutcome::Delivered
    );
}

// ══════════════════════════════════════════════════════════════════════════
// Cobertura complementar: descrição de rotas, roteamento do build (auto,
// sem driver, simulado proibido), atuação real (limites, domínio, falha de
// escrita com retry), potência via RAPL e os braços de resposta do peer
// remoto (READ_OK/READ_ERR/corpo inesperado; ACK de cada AckAct).
// ══════════════════════════════════════════════════════════════════════════
use vbl_fxp::bus::Route;
use vbl_fxp::registry::RemoteAddr;
use vbl_fxp::schema::{reason, Message};
use vbl_runtime::fxp::ActorLimits;

#[test]
fn descricoes_de_rota_debug_e_acessores() {
    // descrição de cada variante de rota (probe/relatório)
    assert_eq!(Route::Simulator.description(), "simulado (em processo)");
    assert_eq!(Route::Real.description(), "real (driver de arquivo)");
    assert_eq!(
        Route::Remote(RemoteAddr::Unix("/tmp/fxpd.sock".into())).description(),
        "remota (unix:/tmp/fxpd.sock)"
    );
    assert_eq!(
        Route::Remote(RemoteAddr::Tcp {
            host: "127.0.0.1".into(),
            port: 9000
        })
        .description(),
        "remota (tcp:127.0.0.1:9000)"
    );
    assert_eq!(
        Route::Inaccessible {
            reason: "sem hardware".into()
        }
        .description(),
        "inacessível (sem hardware)"
    );

    let (bus, _ledger) = bus_simulated();
    // Debug estrutural (log de diagnóstico)
    let dbg = format!("{bus:?}");
    assert!(dbg.starts_with("FxpBus"), "{dbg}");
    assert!(dbg.contains("rotas"), "{dbg}");
    // acessórios de observação
    assert_eq!(bus.registry_rico().len(), 6); // mínimo §6
    assert!(bus.sim().registry().sensores.contains_key("cpu_temp"));
    assert_eq!(bus.pending_queue(), 0);
    assert_eq!(
        bus.route_of("cpu_temp").map(|r| r.description()),
        Some("simulado (em processo)".into())
    );
    assert_eq!(bus.route_of("nem_existe"), None);
    assert_eq!(bus.disk_bytes_used(), 0);
}

#[test]
fn build_roteia_auto_sem_driver_e_simulado_proibido() {
    let dir = tmpdir("roteamento");
    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\n\
         SensorX.grandeza = luz\nSensorX.mode = real\nSensorX.endpoint = auto\n\
         MotorX.min = 0\nMotorX.max = 100\nMotorX.mode = real\nMotorX.endpoint = auto\n\
         cpu_temp.mode = real\ncpu_temp.endpoint = hwmon_pwm:{}/x\n\
         Fan.mode = real\nFan.endpoint = thermal_zone:{}/y\n\
         attention.mode = real\nattention.endpoint = simulado\n",
        dir.display(),
        dir.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig {
            mode: OperationMode::Hybrid,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );

    // auto-descoberta de nome fora do catálogo do host → inacessível honesto
    assert_eq!(
        bus.route_of("SensorX").map(|r| r.description()),
        Some("inacessível (auto-descoberta não encontrou hardware)".into())
    );
    assert_eq!(
        bus.route_of("MotorX").map(|r| r.description()),
        Some("inacessível (auto-descoberta não encontrou hardware)".into())
    );
    // endpoint de ESCRITA num sensor → sem driver de leitura
    assert!(bus
        .route_of("cpu_temp")
        .map(|r| r.description())
        .unwrap()
        .contains("sem driver de leitura"));
    // endpoint de LEITURA num ator → sem driver de atuação
    assert!(bus
        .route_of("Fan")
        .map(|r| r.description())
        .unwrap()
        .contains("sem driver de atuação"));
    // modo real do dispositivo com endpoint simulado: dado sintético proibido
    assert!(bus
        .route_of("attention")
        .map(|r| r.description())
        .unwrap()
        .contains("não roteia para simulador"));

    // leitura em rota inacessível → falha honesta (§4.7)…
    let mut ledger = ChainLedger::new();
    assert_eq!(
        bus.read_sensor("SensorX", &mut ledger),
        Err(SensorFailure::Inaccessible)
    );
    assert!(!ledger.search("ALERT", &[]).is_empty());
    // …e sensor fora do registro → NotRegistered
    assert_eq!(
        bus.read_sensor("nem_existe", &mut ledger),
        Err(SensorFailure::NotRegistered)
    );
}

#[test]
fn ator_em_rota_inacessivel_fallback_ausente_vai_para_a_fila() {
    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse("mode = real\nBomba.min = 0\nBomba.max = 100\n").unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig {
            mode: OperationMode::Real,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    // modo real, ator sem rota real → indisponível; sem fallback → fila
    assert_eq!(
        bus.act("Bomba", Value::Num(10.0), &mut ledger),
        ActOutcome::FallbackExhausted
    );
    assert!(!ledger.search("actor_unavailable", &[]).is_empty());
    assert_eq!(bus.pending_queue(), 1);
    // cpu_power sem rota → simulador embutido (honesto: modo real não lê sim
    // para SENSORES; cpu_power() é só a última potência conhecida)
    let _ = bus.cpu_power();
}

#[test]
fn atuacao_real_limites_inclusivos_dominio_e_falha_de_escrita() {
    let dir = tmpdir("real-ator");
    let pwm = dir.join("pwm1");
    fs::write(&pwm, "0").unwrap();
    fs::create_dir_all(dir.join("tz")).unwrap();
    fs::write(dir.join("cap"), "0").unwrap();
    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\n\
         Fan.mode = real\nFan.endpoint = hwmon_pwm:{}\n\
         Fan.min = 0\nFan.max = 250\nFan.safety_limit = 200\n",
        pwm.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let (mut bus, mut ledger) = (
        FxpBus::build(
            registry,
            BusConfig {
                mode: OperationMode::Hybrid,
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
    );

    // limites INCLUSIVOS do registro validados antes do envio (§4.3)
    assert_eq!(
        bus.act("Fan", Value::Num(200.0), &mut ledger),
        ActOutcome::Delivered // igual ao safety passa (inclusivo), < max
    );
    assert!(matches!(
        bus.act("Fan", Value::Num(251.0), &mut ledger),
        ActOutcome::Rejected {
            limit: Limit::Max,
            ..
        }
    ));
    assert!(matches!(
        bus.act("Fan", Value::Num(-1.0), &mut ledger),
        ActOutcome::Rejected {
            limit: Limit::Min,
            ..
        }
    ));
    // 245 passa no max, estoura o safety → SafetyLimit
    assert!(matches!(
        bus.act("Fan", Value::Num(245.0), &mut ledger),
        ActOutcome::Rejected {
            limit: Limit::SafetyLimit,
            ..
        }
    ));
    assert!(!ledger.search("actor_rejected_value", &[]).is_empty());

    // valor textual fora do domínio numérico do driver → InvalidValue
    assert!(matches!(
        bus.act("Fan", Value::Str("forte".into()), &mut ledger),
        ActOutcome::InvalidValue { .. }
    ));

    // driver some (arquivo removido): retry esgota → indisponível → fila
    fs::remove_file(&pwm).unwrap();
    assert_eq!(
        bus.act("Fan", Value::Num(80.0), &mut ledger),
        ActOutcome::FallbackExhausted
    );
    assert_eq!(bus.pending_queue(), 1);
}

#[test]
fn potencia_real_via_rapl_e_on_tick_silencioso() {
    let dir = tmpdir("rapl-bus");
    fs::write(dir.join("energy_uj"), "1000000").unwrap();
    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\ncache_ttl_ms = 0\ncpu_power.mode = real\ncpu_power.endpoint = rapl_energy:{}",
        dir.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig {
            mode: OperationMode::Hybrid,
            cache_ttl: Duration::ZERO,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    // primeira amostra só inicializa a referência de energia (Δt=0 → err)
    let _ = bus.read_sensor("cpu_power", &mut ledger);
    fs::write(dir.join("energy_uj"), "1003000").unwrap(); // +3000 µJ
    std::thread::sleep(Duration::from_millis(5));
    let p = bus.read_sensor("cpu_power", &mut ledger).unwrap();
    assert!(p > 0.0, "potência real: {p}");
    // cpu_power() devolve a última potência conhecida da rota real
    assert_eq!(bus.cpu_power(), p);
    // on_tick varre a potência silenciosamente (sem Caderno)
    bus.on_tick(&mut ledger);
    let known = bus.cpu_power();
    assert!(known >= 0.0, "potência real conhecida: {known}");
    // fonte some: potência fica inacessível, última conhecida permanece
    fs::remove_file(dir.join("energy_uj")).unwrap();
    bus.on_tick(&mut ledger);
    assert_eq!(bus.cpu_power(), known);
    assert_eq!(
        bus.read_sensor("cpu_power", &mut ledger),
        Err(SensorFailure::Inaccessible)
    );
}

// ---------------------------------------------------------------------------
// Peer remoto: cada braço de resposta do schema v1
// ---------------------------------------------------------------------------

fn bus_remote(sock: &Path, extra_cfg: &str) -> (FxpBus, ChainLedger) {
    let mut registry = DeviceRegistry::minimum();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\ncache_ttl_ms = 0\ncache_ttl_ms = 0\n\
         solar_panel.grandeza = luz\nsolar_panel.unidade = W/m2\n\
         solar_panel.mode = real\nsolar_panel.endpoint = unix:{}\n\
         Bomba.mode = real\nBomba.endpoint = unix:{}\nBomba.min = 0\nBomba.max = 100\n{extra_cfg}",
        sock.display(),
        sock.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    (
        FxpBus::build(
            registry,
            BusConfig {
                mode: OperationMode::Hybrid,
                read_timeout: Duration::from_millis(300),
                act_timeout_local: Duration::from_millis(300),
                ..Default::default()
            },
            vbl_runtime::FxpSimulator::new(),
        ),
        ChainLedger::new(),
    )
}

#[test]
fn peer_read_err_not_registered_e_inacessivel() {
    let sock = tmpdir("readerr").join("fxpd.sock");
    let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let r = reads.clone();
    let _srv = serve_unix(&sock, move |msg| match msg.opcode {
        vbl_fxp::schema::op::READ => Some(match r.fetch_add(1, Ordering::SeqCst) {
            0 => Message::read_err(reason::NOT_REGISTERED, msg.seq),
            _ => Message::read_err(reason::INACCESSIBLE, msg.seq),
        }),
        _ => None, // HELLO não pede resposta no teste
    })
    .expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&sock, "");

    // NOT_REGISTERED → falha tipada sem alerta de I/O
    assert_eq!(
        bus.read_sensor("solar_panel", &mut ledger),
        Err(SensorFailure::NotRegistered)
    );
    assert!(!ledger.search("ALERT", &[]).is_empty());
    // INACCESSIBLE (outro motivo) → Inaccessible + alerta
    assert_eq!(
        bus.read_sensor("solar_panel", &mut ledger),
        Err(SensorFailure::Inaccessible)
    );
    assert!(!ledger.search("ALERT", &[]).is_empty());
}

#[test]
fn peer_resposta_inesperada_e_sintetica_marcada() {
    let sock = tmpdir("inesperado").join("fxpd.sock");
    let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let r = reads.clone();
    let _srv = serve_unix(&sock, move |msg| match msg.opcode {
        vbl_fxp::schema::op::READ => Some(match r.fetch_add(1, Ordering::SeqCst) {
            // corpo errado para um READ (ack de atuação) → resposta inesperada
            0 => Message::act_ack(AckAct::Delivered, false, msg.seq),
            // dado sintético: leitura ok, mas MARCADA no Caderno (§4.7)
            _ => Message::read_ok(42.0, "solar_panel", true, msg.seq),
        }),
        _ => None,
    })
    .expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&sock, "");

    assert_eq!(
        bus.read_sensor("solar_panel", &mut ledger),
        Err(SensorFailure::Inaccessible)
    );
    assert_eq!(bus.read_sensor("solar_panel", &mut ledger).unwrap(), 42.0);
    assert!(!ledger.search("ASSESSMENT", &[]).is_empty());
}

#[test]
fn peer_read_timeout_cai_fora_e_descarta_conexao() {
    let sock = tmpdir("timeout-read").join("fxpd.sock");
    let _srv = serve_unix(&sock, |_| None).expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&sock, "");

    assert_eq!(
        bus.read_sensor("solar_panel", &mut ledger),
        Err(SensorFailure::Inaccessible)
    );
    assert!(!ledger.search("ALERT", &[]).is_empty());
}

#[test]
fn peer_act_todos_os_acks() {
    // um servidor por cenário, respostas determinísticas por requisição
    struct Cenario {
        sock: PathBuf,
        _srv: vbl_fxp::transport::Server,
    }
    fn servidor(
        sock: &Path,
        resposta: impl Fn(u32) -> Message + Send + Sync + Clone + 'static,
    ) -> Cenario {
        let s = sock.to_path_buf();
        let srv = serve_unix(sock, move |msg| Some(resposta(msg.seq))).expect("servidor");
        Cenario { sock: s, _srv: srv }
    }

    let dir = tmpdir("peer-act");
    // 1) Rejected do peer (limite violado LÁ) → Rejected terminativo local
    let c = servidor(&dir.join("a.sock"), |seq| {
        Message::act_ack(
            AckAct::Rejected {
                limit: 1,
                limit_value: 50.0,
            },
            false,
            seq,
        )
    });
    assert!(wait_ready_unix(&c.sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&c.sock, "");
    assert!(matches!(
        bus.act("Bomba", Value::Num(30.0), &mut ledger),
        ActOutcome::Rejected {
            limit: Limit::Max,
            limit_value: 50.0
        }
    ));
    drop(c);

    // 2) MissingActor do peer → terminativo, sem fila
    let c = servidor(&dir.join("b.sock"), |seq| {
        Message::act_ack(AckAct::MissingActor, false, seq)
    });
    assert!(wait_ready_unix(&c.sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&c.sock, "");
    assert_eq!(
        bus.act("Bomba", Value::Num(30.0), &mut ledger),
        ActOutcome::MissingActor
    );
    assert_eq!(bus.pending_queue(), 0);
    drop(c);

    // 3) InvalidValue do peer
    let c = servidor(&dir.join("c.sock"), |seq| {
        Message::act_ack(
            AckAct::InvalidValue {
                reason: "fora do domínio".into(),
            },
            false,
            seq,
        )
    });
    assert!(wait_ready_unix(&c.sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&c.sock, "");
    assert!(matches!(
        bus.act("Bomba", Value::Num(30.0), &mut ledger),
        ActOutcome::InvalidValue { .. }
    ));
    drop(c);

    // 4) Unavailable do peer → indisponível → fallback esgotado → fila
    let c = servidor(&dir.join("d.sock"), |seq| {
        Message::act_ack(AckAct::Unavailable, false, seq)
    });
    assert!(wait_ready_unix(&c.sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&c.sock, "");
    assert_eq!(
        bus.act("Bomba", Value::Num(30.0), &mut ledger),
        ActOutcome::FallbackExhausted
    );
    assert_eq!(bus.pending_queue(), 1);
    drop(c);

    // 5) FallbackExecuted do peer: o PEER acionou o fallback DELE
    let c = servidor(&dir.join("e.sock"), |seq| {
        Message::act_ack(
            AckAct::FallbackExecuted {
                alternativo: "BombaBackup".into(),
            },
            false,
            seq,
        )
    });
    assert!(wait_ready_unix(&c.sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&c.sock, "");
    assert_eq!(
        bus.act("Bomba", Value::Num(30.0), &mut ledger),
        ActOutcome::FallbackExecuted {
            alternativo: "BombaBackup".into()
        }
    );
    drop(c);

    // 6) corpo inesperado para ACT (read_ok) → indisponível → fila
    let c = servidor(&dir.join("f.sock"), |seq| {
        Message::read_ok(1.0, "qualquer", false, seq)
    });
    assert!(wait_ready_unix(&c.sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&c.sock, "");
    assert_eq!(
        bus.act("Bomba", Value::Num(30.0), &mut ledger),
        ActOutcome::FallbackExhausted
    );
    drop(c);

    // 7) valor textual via remota: serializa como string no fio
    let c = servidor(&dir.join("g.sock"), |seq| {
        Message::act_ack(AckAct::Delivered, false, seq)
    });
    assert!(wait_ready_unix(&c.sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&c.sock, "");
    assert_eq!(
        bus.act("Bomba", Value::Str("ligar".into()), &mut ledger),
        ActOutcome::Delivered
    );
}

#[test]
fn ator_remoto_com_limites_do_registro_rejeita_localmente() {
    // limites vêm do registro rico — inclusive em rota remota
    let sock = tmpdir("remoto-limites").join("fxpd.sock");
    let _srv = serve_unix(&sock, |msg| {
        Some(Message::act_ack(AckAct::Delivered, false, msg.seq))
    })
    .expect("servidor");
    assert!(wait_ready_unix(&sock, DEADLINE));
    let (mut bus, mut ledger) = bus_remote(&sock, "Bomba.max = 10\n");

    assert!(matches!(
        bus.act("Bomba", Value::Num(11.0), &mut ledger),
        ActOutcome::Rejected {
            limit: Limit::Max,
            limit_value: 10.0
        }
    ));
    // ator além do limite do PEER (bomba do peer rejeita) coberto em peer_act
    let _ = ActorLimits::default();
}
