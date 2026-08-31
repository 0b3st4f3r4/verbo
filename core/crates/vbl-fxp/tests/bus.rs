//! Integração do `FxpBus`: modos real/simulado/híbrido, honestidade de dados
//! (FORMAL §4.7), cache, fallback do registro (§4.3), fila prioritária com
//! re-entrega/expiração, transporte remoto e o cenário capitular da Etapa 3:
//! subversão térmica E2E com drivers reais em fixtures.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::bus::{BusConfig, FxpBus};
use vbl_fxp::registry::{DeviceRegistry, FxpConfig, OperationMode};
use vbl_fxp::queue::PRIORITY_SUBVERT;
use vbl_fxp::schema::AckAct;
use vbl_fxp::transport::{wait_ready_unix, serve_unix};
use vbl_runtime::ledger::ChainLedger;
use vbl_runtime::fxp::{
    ActOutcome, SensorFailure, Fxp, Limit, Value,
};
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
    let cfg = BusConfig { mode: OperationMode::Simulated, ..Default::default() };
    (
        FxpBus::build(DeviceRegistry::minimum(), cfg, vbl_runtime::FxpSimulator::new()),
        ChainLedger::new(),
    )
}

/// Bus híbrido com cpu_temp (thermal), Ventoinha (pwm) e CpuPowerCap (cap)
/// em rotas REAIS de fixture. Recebe os CAMINHOS DOS ARQUIVOS de pwm/cap.
fn bus_hybrid_fixture(tz_dir: &Path, pwm_file: &Path, cap_file: &Path) -> (FxpBus, ChainLedger) {
    let cfg_text = format!(
        "mode = hibrido\n\
         cache_ttl_ms = 0\n\
         cpu_temp.mode = real\n\
         cpu_temp.endpoint = thermal_zone:{}\n\
         Ventoinha.mode = real\n\
         Ventoinha.endpoint = hwmon_pwm:{}\n\
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
        ActOutcome::Rejected { limit: Limit::SafetyLimit, limit_value: 200.0 }
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
    let cfg = BusConfig { mode: OperationMode::Real, ..Default::default() };
    let (mut bus, mut ledger) = (
        FxpBus::build(DeviceRegistry::minimum(), cfg, vbl_runtime::FxpSimulator::new()),
        ChainLedger::new(),
    );
    assert_eq!(
        bus.read_sensor("attention", &mut ledger),
        Err(SensorFailure::Inaccessible)
    );
    assert!(ledger.search("ALERT", &[]).iter().any(|e| e.msg.contains("attention")));
    // A rota documenta o motivo (honestidade observável).
    assert!(bus.route_of("attention").unwrap().description().contains("inacessível"));
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
        BusConfig { mode: OperationMode::Hybrid, ..Default::default() },
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
nonequilibrium TradingEspeculativo {
    value: "lucro_arbitragem_alta_frequencia",
    horizon: 7s,
    source_path: "cpu_temp",
    maintenance_deadline: 2s,
    exchange_mode: "extraction"
}

review TradingEspeculativo {
    when cpu_temp > 85°C -> subvert,
                            act(CpuPowerCap, 50)
}
"#;
    let (program, diags) = vbl_lang::parse(source);
    assert!(diags.errors().next().is_none(), "programa deve parsear: {diags:?}");
    let _ = load(&mut engine, &program);

    engine.tick();

    // 1) subvert dissolve no mesmo tick (§4.5).
    assert!(engine.form("TradingEspeculativo").is_none(), "forma subvertida deve dissolver no tick");
    // 2) a act pós-subvert chegou ao DRIVER REAL em µW (50 W).
    let written = fs::read_to_string(&cap).unwrap();
    assert_eq!(written, "50000000", "CpuPowerCap real deve receber 50 W em µW");
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
         Ventoinha.mode = real\nVentoinha.endpoint = hwmon_pwm:{}\n\
         VentoinhaReserva.mode = real\nVentoinhaReserva.endpoint = hwmon_pwm:{}\n\
         VentoinhaReserva.min = 0\nVentoinhaReserva.max = 255\n\
         fallback.Ventoinha = VentoinhaReserva\n",
        prim.display(),
        alt.display()
    ))
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let mut bus = FxpBus::build(
        registry,
        BusConfig { mode: OperationMode::Hybrid, ..Default::default() },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    // Primário OK: entrega direta no pwm1.
    assert_eq!(bus.act("Ventoinha", Value::Num(200.0), &mut ledger), ActOutcome::Delivered);
    assert_eq!(fs::read_to_string(&prim).unwrap(), "200");

    // Primário morre (endpoint removido) → fallback do registro → pwm2.
    fs::remove_file(&prim).unwrap();
    let outcome = bus.act("Ventoinha", Value::Num(180.0), &mut ledger);
    assert!(matches!(outcome,
        ActOutcome::FallbackExecuted { ref alternativo } if alternativo == "VentoinhaReserva"));
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
         Ventoinha.mode = real\nVentoinha.endpoint = hwmon_pwm:{}\n",
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
    let outcome = bus.act_with_priority("Ventoinha", Value::Num(120.0), PRIORITY_SUBVERT, &mut ledger);
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
    let outcome = bus.act("Ventoinha", Value::Num(60.0), &mut ledger);
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

    assert_eq!(bus.read_sensor("human_attention", &mut ledger).unwrap(), 100.0);
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
    let _srv = serve_unix(
        &sock,
        |msg| match msg.opcode {
            vbl_fxp::schema::op::READ => {
                Some(vbl_fxp::Message::read_ok(77.5, "solar_panel", false, msg.seq))
            }
            vbl_fxp::schema::op::ACT => {
                Some(vbl_fxp::Message::act_ack(AckAct::Delivered, false, msg.seq))
            }
            _ => None,
        },
    )
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
        BusConfig { mode: OperationMode::Hybrid, ..Default::default() },
        vbl_runtime::FxpSimulator::new(),
    );
    let mut ledger = ChainLedger::new();

    assert_eq!(bus.read_sensor("solar_panel", &mut ledger).unwrap(), 77.5);
    assert_eq!(bus.act("Bomba", Value::Num(50.0), &mut ledger), ActOutcome::Delivered);
    // Limite validado LOCALMENTE antes do envio (§4.3).
    assert!(matches!(
        bus.act("Bomba", Value::Num(101.0), &mut ledger),
        ActOutcome::Rejected { limit: Limit::Max, limit_value: 100.0 }
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
        "VentoinhaReserva.min = 0\nVentoinhaReserva.max = 255\n\
         fallback.Ventoinha = VentoinhaReserva\n",
    )
    .unwrap();
    cfg.apply(&mut registry).unwrap();
    let (mut bus, mut ledger) = (
        FxpBus::build(registry, BusConfig::default(), vbl_runtime::FxpSimulator::new()),
        ChainLedger::new(),
    );

    // Runtime registry (validação do loader) inclui a extensão…
    assert!(bus.registry().actors.contains_key("VentoinhaReserva"));
    // …e o backend simulado roteia a extensão com os limites do registro.
    assert!(matches!(
        bus.act("VentoinhaReserva", Value::Num(300.0), &mut ledger),
        ActOutcome::Rejected { limit: Limit::Max, limit_value: 255.0 }
    ));
    assert_eq!(
        bus.act("VentoinhaReserva", Value::Num(255.0), &mut ledger),
        ActOutcome::Delivered
    );
}

