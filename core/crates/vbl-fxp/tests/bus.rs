//! Integração do `FxpBus`: modos real/simulado/híbrido, honestidade de dados
//! (FORMAL §4.7), cache, fallback do registro (§4.3), fila prioritária com
//! re-entrega/expiração, transporte remoto e o cenário capitular da Etapa 3:
//! subversão térmica E2E com drivers reais em fixtures.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vbl_fxp::bus::{BusConfig, FxpBus};
use vbl_fxp::registry::{DeviceRegistry, FxpConfig, ModoOperacao};
use vbl_fxp::queue::PRIORIDADE_SUBVERT;
use vbl_fxp::schema::AckAct;
use vbl_fxp::transport::{esperar_pronto_unix, servir_unix};
use vbl_runtime::notebook::ChainCaderno;
use vbl_runtime::fxp::{
    ActOutcome, FalhaSensor, Fxp, Limite, Value,
};
use vbl_runtime::{carregar, Engine};

const PRAZO: Duration = Duration::from_secs(2);

fn tmpdir(nome: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vbl-bus-{}-{}-{}",
        nome,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("criar tmpdir");
    dir
}

/// thermal_zone sintética: escreve `temp` em mili°C.
fn fixture_thermal(dir: &Path, milic: i64) {
    fs::write(dir.join("temp"), milic.to_string()).unwrap();
}

fn bus_simulado() -> (FxpBus, ChainCaderno) {
    let cfg = BusConfig { modo: ModoOperacao::Simulado, ..Default::default() };
    (
        FxpBus::construir(DeviceRegistry::minimo(), cfg, vbl_runtime::FxpSimulator::novo()),
        ChainCaderno::new(),
    )
}

/// Bus híbrido com cpu_temp (thermal), Ventoinha (pwm) e CpuPowerCap (cap)
/// em rotas REAIS de fixture. Recebe os CAMINHOS DOS ARQUIVOS de pwm/cap.
fn bus_hibrido_fixture(tz_dir: &Path, pwm_file: &Path, cap_file: &Path) -> (FxpBus, ChainCaderno) {
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
    let mut registry = DeviceRegistry::minimo();
    cfg.aplicar(&mut registry).unwrap();
    let bus_cfg = BusConfig {
        modo: ModoOperacao::Hibrido,
        cache_ttl: Duration::ZERO, // determinismo nos testes
        ..Default::default()
    };
    (
        FxpBus::construir(registry, bus_cfg, vbl_runtime::FxpSimulator::novo()),
        ChainCaderno::new(),
    )
}

// ---------------------------------------------------------------------------
// Modo simulado: paridade com a Etapa 2
// ---------------------------------------------------------------------------

#[test]
fn modo_simulado_tem_paridade_com_a_etapa_2() {
    let (mut bus, mut caderno) = bus_simulado();

    // Leitura: valores plausíveis do simulador.
    assert_eq!(bus.read_sensor("cpu_temp", &mut caderno).unwrap(), 55.0);
    bus.sim_mut().set_sensor("cpu_temp", 86.5);
    assert_eq!(bus.read_sensor("cpu_temp", &mut caderno).unwrap(), 86.5);

    // Limites inclusivos: 200 passa; 200.1 e 201 violam o safety_limit.
    assert_eq!(
        bus.act("CpuPowerCap", Value::Num(200.0), &mut caderno),
        ActOutcome::Entregue
    );
    assert!(matches!(
        bus.act("CpuPowerCap", Value::Num(200.1), &mut caderno),
        ActOutcome::Rejeitado { limite: Limite::SafetyLimit, valor_limite: 200.0 }
    ));

    // cpu_power para partilha P/N vem do simulador.
    assert_eq!(bus.cpu_power(), 150.0);

    // Sensor ausente: nunca 0.0 (§4.7).
    assert_eq!(
        bus.read_sensor("nao_existe", &mut caderno),
        Err(FalhaSensor::NaoRegistrado)
    );
}

// ---------------------------------------------------------------------------
// Híbrido: rotas reais em fixtures
// ---------------------------------------------------------------------------

#[test]
fn hibrido_le_driver_real_e_converte_unidade() {
    let tz = tmpdir("tz");
    fixture_thermal(&tz, 86_500);
    let pwm = tmpdir("pwm").join("pwm1");
    let cap = tmpdir("cap").join("constraint_0_power_limit_uw");
    fs::write(&pwm, "0").unwrap();
    fs::write(&cap, "250000000").unwrap();

    let (mut bus, mut caderno) = bus_hibrido_fixture(&tz, &pwm, &cap);

    // mili°C → °C exato.
    assert_eq!(bus.read_sensor("cpu_temp", &mut caderno).unwrap(), 86.5);
    // attention continua simulada (não configurada como real).
    assert_eq!(bus.read_sensor("attention", &mut caderno).unwrap(), 100.0);
}

#[test]
fn modo_real_sem_rota_e_inacessivel_nunca_fabricado() {
    // Global REAL: attention (simulada no registro) fica inacessível — o bus
    // não fabrica leitura sintética em modo real (§4.7).
    let cfg = BusConfig { modo: ModoOperacao::Real, ..Default::default() };
    let (mut bus, mut caderno) = (
        FxpBus::construir(DeviceRegistry::minimo(), cfg, vbl_runtime::FxpSimulator::novo()),
        ChainCaderno::new(),
    );
    assert_eq!(
        bus.read_sensor("attention", &mut caderno),
        Err(FalhaSensor::Inacessivel)
    );
    assert!(caderno.buscar("ALERTA", &[]).iter().any(|e| e.msg.contains("attention")));
    // A rota documenta o motivo (honestidade observável).
    assert!(bus.rota_de("attention").unwrap().descricao().contains("inacessível"));
}

#[test]
fn cache_ttl_poupa_leituras_reais() {
    let tz = tmpdir("tz-cache");
    fixture_thermal(&tz, 50_000);
    let cfg_text = format!(
        "mode = hibrido\ncache_ttl_ms = 60000\ncpu_temp.mode = real\ncpu_temp.endpoint = thermal_zone:{}\n",
        tz.display()
    );
    let cfg = FxpConfig::parse(&cfg_text).unwrap();
    let mut registry = DeviceRegistry::minimo();
    cfg.aplicar(&mut registry).unwrap();
    let mut bus = FxpBus::construir(
        registry,
        BusConfig { modo: ModoOperacao::Hibrido, ..Default::default() },
        vbl_runtime::FxpSimulator::novo(),
    );
    let mut caderno = ChainCaderno::new();

    assert_eq!(bus.read_sensor("cpu_temp", &mut caderno).unwrap(), 50.0);
    // Mundo muda; cache (TTL 60 s) ainda vale → leitura envelhecida.
    fixture_thermal(&tz, 90_000);
    assert_eq!(bus.read_sensor("cpu_temp", &mut caderno).unwrap(), 50.0);
}

// ---------------------------------------------------------------------------
// Cenário capitular: subversão térmica E2E com drivers reais
// ---------------------------------------------------------------------------

#[test]
fn subversao_termica_e2e_com_drivers_reais() {
    // BDD Caso 2 na Etapa 3: `when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50)`
    // sobre o BUS híbrido com cpu_temp e CpuPowerCap REAIS (fixtures sysfs).
    let tz = tmpdir("tz-subvert");
    fixture_thermal(&tz, 86_500); // 86.5°C > 85°C
    let cap = tmpdir("cap-subvert").join("constraint_0_power_limit_uw");
    fs::write(&cap, "250000000").unwrap();
    let pwm = tmpdir("pwm-subvert").join("pwm1");
    fs::write(&pwm, "0").unwrap();

    let (bus, _) = bus_hibrido_fixture(&tz, &pwm, &cap);
    let mut engine = Engine::novo(bus, 1.0, tmpdir("persist-subvert"));

    let fonte = r#"
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
    let (programa, diags) = vbl_lang::parse(fonte);
    assert!(diags.errors().next().is_none(), "programa deve parsear: {diags:?}");
    let _ = carregar(&mut engine, &programa);

    engine.tick();

    // 1) subvert dissolve no mesmo tick (§4.5).
    assert!(engine.forma("TradingEspeculativo").is_none(), "forma subvertida deve dissolver no tick");
    // 2) a act pós-subvert chegou ao DRIVER REAL em µW (50 W).
    let escrito = fs::read_to_string(&cap).unwrap();
    assert_eq!(escrito, "50000000", "CpuPowerCap real deve receber 50 W em µW");
    // 3) trilha no Caderno: ATUACAO sucesso + SUBVERSAO.
    assert!(engine
        .caderno
        .buscar("ATUACAO", &[])
        .iter()
        .any(|e| e.msg.contains("CpuPowerCap") && e.msg.contains("sucesso")));
    assert!(!engine.caderno.buscar("SUBVERSAO", &[]).is_empty());
    // 4) valor poético canônico aplicado (§4.5).
    assert!(!engine.caderno.buscar("dissolve_subvert", &[]).is_empty());
}

// ---------------------------------------------------------------------------
// Fallback do registro (§4.3) com rotas reais
// ---------------------------------------------------------------------------

#[test]
fn fallback_real_ator_alternativo_e_trilha_completa() {
    let dir = tmpdir("fallback");
    let prim = dir.join("pwm1");
    let alt = dir.join("pwm2");
    fs::write(&prim, "0").unwrap();
    fs::write(&alt, "0").unwrap();

    let mut registry = DeviceRegistry::minimo();
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
    cfg.aplicar(&mut registry).unwrap();
    let mut bus = FxpBus::construir(
        registry,
        BusConfig { modo: ModoOperacao::Hibrido, ..Default::default() },
        vbl_runtime::FxpSimulator::novo(),
    );
    let mut caderno = ChainCaderno::new();

    // Primário OK: entrega direta no pwm1.
    assert_eq!(bus.act("Ventoinha", Value::Num(200.0), &mut caderno), ActOutcome::Entregue);
    assert_eq!(fs::read_to_string(&prim).unwrap(), "200");

    // Primário morre (endpoint removido) → fallback do registro → pwm2.
    fs::remove_file(&prim).unwrap();
    let outcome = bus.act("Ventoinha", Value::Num(180.0), &mut caderno);
    assert!(matches!(outcome,
        ActOutcome::FallbackExecutado { ref alternativo } if alternativo == "VentoinhaReserva"));
    assert_eq!(fs::read_to_string(&alt).unwrap(), "180");

    // Trilha completa: indisponível + fallback executado.
    assert!(!caderno.buscar("ator_indisponivel", &[]).is_empty());
    assert!(!caderno.buscar("fallback_executado", &[]).is_empty());
}

// ---------------------------------------------------------------------------
// Fila prioritária: re-entrega e expiração no relógio virtual
// ---------------------------------------------------------------------------

#[test]
fn fila_reentrega_e_expira_com_auditoria() {
    let dir = tmpdir("fila");
    let prim = dir.join("pwm1");
    fs::write(&prim, "0").unwrap();

    let mut registry = DeviceRegistry::minimo();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\n\
         Ventoinha.mode = real\nVentoinha.endpoint = hwmon_pwm:{}\n",
        prim.display()
    ))
    .unwrap();
    cfg.aplicar(&mut registry).unwrap();
    let mut bus = FxpBus::construir(
        registry,
        BusConfig {
            modo: ModoOperacao::Hibrido,
            queue_timeout_ticks: 2,
            retries: 0, // determinismo: uma tentativa por tick
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::novo(),
    );
    let mut caderno = ChainCaderno::new();

    // Ator morto: esgota → comando entra na fila com prioridade de subvert.
    fs::remove_file(&prim).unwrap();
    let outcome = bus.act_with_priority("Ventoinha", Value::Num(120.0), PRIORIDADE_SUBVERT, &mut caderno);
    assert_eq!(outcome, ActOutcome::FallbackEsgotado);
    assert_eq!(bus.fila_pendente(), 1);

    // Tick 1: endpoint ainda morto → continua pendente (0 < 2, devolvido com 1).
    bus.on_tick(&mut caderno);
    assert_eq!(bus.fila_pendente(), 1);

    // Ator volta → tick 2 re-entrega.
    fs::write(&prim, "0").unwrap();
    bus.on_tick(&mut caderno);
    assert_eq!(bus.fila_pendente(), 0);
    assert_eq!(fs::read_to_string(&prim).unwrap(), "120");
    assert!(!caderno.buscar("comando_reentregue", &[]).is_empty());

    // Expiração: com o ator morto de novo, o comando pendente é descartado
    // no tick que atinge queue_timeout_ticks (2), com evento + alerta.
    fs::remove_file(&prim).unwrap();
    let outcome = bus.act("Ventoinha", Value::Num(60.0), &mut caderno);
    assert_eq!(outcome, ActOutcome::FallbackEsgotado);
    bus.on_tick(&mut caderno); // ticks_esperando 0→1
    bus.on_tick(&mut caderno); // 1→2
    bus.on_tick(&mut caderno); // 2 >= 2 → expira
    assert_eq!(bus.fila_pendente(), 0);
    assert!(!caderno.buscar("comando_expirado", &[]).is_empty());
    assert!(caderno
        .buscar("ALERTA", &[])
        .iter()
        .any(|e| e.msg.contains("expirou")));
}

// ---------------------------------------------------------------------------
// Alias no Caderno: nome usado + canônico (FORMAL §6)
// ---------------------------------------------------------------------------

#[test]
fn leitura_por_alias_registra_usado_e_canonico() {
    let mut registry = DeviceRegistry::minimo();
    registry.definir_alias("human_attention", "attention").unwrap();
    let mut bus = FxpBus::construir(
        registry,
        BusConfig::default(),
        vbl_runtime::FxpSimulator::novo(),
    );
    let mut caderno = ChainCaderno::new();

    assert_eq!(bus.read_sensor("human_attention", &mut caderno).unwrap(), 100.0);
    // Evento de mapeamento com o canônico (a LEITURA com o nome usado é do
    // consumidor — no engine — e é coberta no teste E2E).
    assert!(caderno
        .buscar("INFO", &[])
        .iter()
        .any(|e| e.msg.contains("attention") && e.msg.contains("canônico")));

    // Alias de sensor não serve de ator (kinds são distintos).
    assert_eq!(
        bus.act("human_attention", Value::Num(1.0), &mut caderno),
        ActOutcome::AtorInexistente
    );
}

// ---------------------------------------------------------------------------
// Rota remota: schema v1 sobre Unix com servidor de referência
// ---------------------------------------------------------------------------

#[test]
fn rota_remota_sensor_e_ator_via_schema_v1() {
    let sock = tmpdir("remoto").join("fxpd.sock");
    let _srv = servir_unix(
        &sock,
        |msg| match msg.opcode {
            vbl_fxp::schema::op::READ => {
                Some(vbl_fxp::Mensagem::read_ok(77.5, "solar_panel", false, msg.seq))
            }
            vbl_fxp::schema::op::ACT => {
                Some(vbl_fxp::Mensagem::act_ack(AckAct::Entregue, false, msg.seq))
            }
            _ => None,
        },
    )
    .expect("servidor");
    assert!(esperar_pronto_unix(&sock, PRAZO));

    let mut registry = DeviceRegistry::minimo();
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
    cfg.aplicar(&mut registry).unwrap();
    let mut bus = FxpBus::construir(
        registry,
        BusConfig { modo: ModoOperacao::Hibrido, ..Default::default() },
        vbl_runtime::FxpSimulator::novo(),
    );
    let mut caderno = ChainCaderno::new();

    assert_eq!(bus.read_sensor("solar_panel", &mut caderno).unwrap(), 77.5);
    assert_eq!(bus.act("Bomba", Value::Num(50.0), &mut caderno), ActOutcome::Entregue);
    // Limite validado LOCALMENTE antes do envio (§4.3).
    assert!(matches!(
        bus.act("Bomba", Value::Num(101.0), &mut caderno),
        ActOutcome::Rejeitado { limite: Limite::Max, valor_limite: 100.0 }
    ));
}

#[test]
fn ator_remoto_mudo_vira_indisponivel_e_fila() {
    let sock = tmpdir("mudo2").join("fxpd.sock");
    let _srv = servir_unix(&sock, |_| None).expect("servidor");
    assert!(esperar_pronto_unix(&sock, PRAZO));

    let mut registry = DeviceRegistry::minimo();
    let cfg = FxpConfig::parse(&format!(
        "mode = hibrido\n\
         Bomba.mode = real\nBomba.endpoint = unix:{}\nBomba.min = 0\nBomba.max = 100\n",
        sock.display()
    ))
    .unwrap();
    cfg.aplicar(&mut registry).unwrap();
    let mut bus = FxpBus::construir(
        registry,
        BusConfig {
            modo: ModoOperacao::Hibrido,
            act_timeout_local: Duration::from_millis(100),
            queue_timeout_ticks: 1,
            ..Default::default()
        },
        vbl_runtime::FxpSimulator::novo(),
    );
    let mut caderno = ChainCaderno::new();

    let outcome = bus.act("Bomba", Value::Num(10.0), &mut caderno);
    assert_eq!(outcome, ActOutcome::FallbackEsgotado);
    assert!(!caderno.buscar("ator_indisponivel", &[]).is_empty());
    assert_eq!(bus.fila_pendente(), 1);

    // tick 1: mudo de novo → re-enfileirado com 1 tick de espera…
    bus.on_tick(&mut caderno);
    assert_eq!(bus.fila_pendente(), 1);
    // …tick 2: 1 >= queue_timeout_ticks(1) → expira com auditoria.
    bus.on_tick(&mut caderno);
    assert_eq!(bus.fila_pendente(), 0);
    assert!(!caderno.buscar("comando_expirado", &[]).is_empty());
}

// ---------------------------------------------------------------------------
// Extensões do registro chegam ao backend simulado (fonte única)
// ---------------------------------------------------------------------------

#[test]
fn extensao_do_registro_e_visivel_no_simulador_e_no_runtime() {
    let mut registry = DeviceRegistry::minimo();
    let cfg = FxpConfig::parse(
        "VentoinhaReserva.min = 0\nVentoinhaReserva.max = 255\n\
         fallback.Ventoinha = VentoinhaReserva\n",
    )
    .unwrap();
    cfg.aplicar(&mut registry).unwrap();
    let (mut bus, mut caderno) = (
        FxpBus::construir(registry, BusConfig::default(), vbl_runtime::FxpSimulator::novo()),
        ChainCaderno::new(),
    );

    // Runtime registry (validação do loader) inclui a extensão…
    assert!(bus.registry().atores.contains_key("VentoinhaReserva"));
    // …e o backend simulado roteia a extensão com os limites do registro.
    assert!(matches!(
        bus.act("VentoinhaReserva", Value::Num(300.0), &mut caderno),
        ActOutcome::Rejeitado { limite: Limite::Max, valor_limite: 255.0 }
    ));
    assert_eq!(
        bus.act("VentoinhaReserva", Value::Num(255.0), &mut caderno),
        ActOutcome::Entregue
    );
}

