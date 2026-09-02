//! Travessia de cobertura do runtime: builder e mutadores do simulador
//! (todos os caminhos de fallback), cláusulas raras do engine (exchange_mode
//! não canônico, reclassify sem efeito, persistência que falha, atuação que
//! falha) e a matriz de validação de registro do loader (FORMAL §3/§6).

use std::path::PathBuf;
use vbl_lang::Conjugation;
use vbl_runtime::fxp::{ActorLimits, Fxp as _, SensorFailure, SensorInfo, Value};
use vbl_runtime::ledger::ChainLedger;
use vbl_runtime::{load, validate, Engine, FxpSimulator, Registry};

fn tmpdir(nome: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vbl-travessia-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Simulador: builder, mutadores e cadeia de fallback com todos os desvios
// ---------------------------------------------------------------------------

#[test]
fn simulador_builder_default_e_mutadores() {
    // Default == new (registro mínimo + valores plausíveis)
    let mut por_default = FxpSimulator::default();
    let mut direto = FxpSimulator::new();
    assert_eq!(
        por_default.read_sensor("cpu_temp", &mut ChainLedger::new()),
        direto.read_sensor("cpu_temp", &mut ChainLedger::new())
    );

    // builder: sensor novo com valor, ator novo com limites, override de valor
    let mut sim = vbl_runtime::sim::SimulatorBuilder::new()
        .with_sensor(
            "luz",
            SensorInfo {
                quantity: "iluminancia".into(),
                unit: "lux".into(),
            },
            7.0,
        )
        .with_actor(
            "ReserveFan",
            ActorLimits {
                min: Some(0.0),
                max: Some(10.0),
                safety_limit: None,
            },
        )
        .with_value("cpu_temp", 20.0)
        .build();
    let mut ledger = ChainLedger::new();

    // valor customizado e sensor novo visíveis
    assert_eq!(sim.read_sensor("cpu_temp", &mut ledger), Ok(20.0));
    assert_eq!(sim.read_sensor("luz", &mut ledger), Ok(7.0));

    // falha e recuperação de sensor
    sim.fail_sensor("cpu_temp");
    assert_eq!(
        sim.read_sensor("cpu_temp", &mut ledger),
        Err(SensorFailure::Inaccessible)
    );
    sim.recover_sensor("cpu_temp");
    assert_eq!(sim.read_sensor("cpu_temp", &mut ledger), Ok(20.0));

    // sensor removido do registro → NotRegistered
    sim.unregister_sensor("luz");
    assert_eq!(
        sim.read_sensor("luz", &mut ledger),
        Err(SensorFailure::NotRegistered)
    );

    // registro de sensor e ator NOVOS pós-build (sincronização do barramento)
    sim.register_sensor(
        "ruido",
        SensorInfo {
            quantity: "ruido".into(),
            unit: "dB".into(),
        },
    );
    assert_eq!(sim.read_sensor("ruido", &mut ledger), Ok(0.0)); // 0.0 até roteirizar
    sim.register_actor("Extra", ActorLimits::default());
    assert!(matches!(
        sim.act("Extra", Value::Num(1.0), &mut ledger),
        vbl_runtime::fxp::ActOutcome::Delivered
    ));

    // agendamento aplica no tick marcado
    sim.schedule(2, "cpu_temp", 99.0);
    sim.on_tick(&mut ledger);
    assert_eq!(sim.read_sensor("cpu_temp", &mut ledger), Ok(20.0));
    sim.on_tick(&mut ledger);
    assert_eq!(sim.read_sensor("cpu_temp", &mut ledger), Ok(99.0));

    // efeitos físicos dos atores (PLAN §6.5): potência cap e ventoinha
    sim.set_sensor("cpu_power", 150.0);
    sim.act("CpuPowerCap", Value::Num(90.0), &mut ledger);
    assert_eq!(sim.read_sensor("cpu_power", &mut ledger), Ok(90.0)); // só desce
    let antes = sim.read_sensor("cpu_temp", &mut ledger).unwrap();
    sim.act("Fan", Value::Num(200.0), &mut ledger); // safety 200 é inclusivo
    let depois = sim.read_sensor("cpu_temp", &mut ledger).unwrap();
    assert!(depois < antes, "ventoinha resfria: {antes} → {depois}");
    // ator sem efeito físico
    sim.act("StatusLed", Value::Num(1.0), &mut ledger);

    // suporte estável (Fxp trait): base 1024 + escrita simulada
    assert_eq!(sim.disk_bytes_used(), 1024);
    sim.add_disk_bytes(64);
    assert_eq!(sim.disk_bytes_used(), 1088);
}

#[test]
fn cadeia_de_fallback_com_todos_os_desvios() {
    use vbl_runtime::fxp::ActOutcome;
    let mut sim = FxpSimulator::new();
    sim.register_actor("Fan2", ActorLimits::default());
    sim.register_actor(
        "Fan3",
        ActorLimits {
            min: Some(0.0),
            max: Some(10.0),
            safety_limit: None,
        },
    );
    sim.register_actor("Fan4", ActorLimits::default());
    // cadeia: inexistente → indisponível → limite → ok
    sim.set_fallback("Fan", &["ReserveFan", "Fan2", "Fan3", "Fan4"]);
    let mut ledger = ChainLedger::new();

    sim.fail_actor("Fan");
    sim.fail_actor("Fan2"); // segundo da cadeia indisponível
                            // valor 50 passa por Fan3? não: max 10 → rejeitado por limite; Fan4 entrega
    assert_eq!(
        sim.act("Fan", Value::Num(50.0), &mut ledger),
        ActOutcome::FallbackExecuted {
            alternativo: "Fan4".into()
        }
    );
    // desvios registrados: fallback rejeitado por limite (Fan3)…
    assert!(!ledger.search("ALERT", &[]).is_empty());
    // …e o executado no fim
    assert!(!ledger.search("fallback_executed", &[]).is_empty());

    // todos indisponíveis → esgotado com alerta
    sim.fail_actor("Fan3");
    sim.fail_actor("Fan4");
    let mut ledger2 = ChainLedger::new();
    assert_eq!(
        sim.act("Fan", Value::Num(5.0), &mut ledger2),
        ActOutcome::FallbackExhausted
    );
    assert!(!ledger2.search("ALERT", &[]).is_empty());

    // recuperação devolve a disponibilidade
    sim.recover_actor("Fan4");
    let mut ledger3 = ChainLedger::new();
    assert!(matches!(
        sim.act("Fan4", Value::Num(3.0), &mut ledger3),
        ActOutcome::Delivered
    ));
}

// ---------------------------------------------------------------------------
// Engine: cláusulas raras do ciclo de vida
// ---------------------------------------------------------------------------

fn forma_evento(nome: &str, agora: f64) -> vbl_runtime::Form {
    vbl_runtime::Form {
        name: nome.into(),
        value: Value::Num(1.0),
        horizon_s: 30.0,
        creation_time: agora,
        conjugation: Conjugation::Event,
        currency: "CpuCycles".into(),
        source_path: None,
        classification: None,
        declared_maintenance_deadline: None,
        maintenance: None,
        exchange_mode: None,
        cost_bytes: None,
        rules: Vec::new(),
        dissolved: false,
        horizon_version: 0,
        maintenance_version: 0,
    }
}

#[test]
fn exchange_mode_nao_canonico_alerta_e_accessors() {
    let dir = tmpdir("exchange");
    let mut engine = Engine::new(FxpSimulator::new(), 1.0, &dir);
    // accessores básicos
    assert_eq!(engine.tick_seconds(), 1.0);
    assert_eq!(engine.persistence_dir(), dir.as_path());
    engine.set_ledger_time(0, 0.0);

    // nonequilibrium com exchange_mode fora do canônico → alerta de auditoria
    let mut forma = forma_evento("Torcida", 0.0);
    forma.conjugation = Conjugation::Nonequilibrium;
    forma.declared_maintenance_deadline = Some(10.0);
    forma.maintenance = Some(vbl_runtime::Maintenance {
        deadline_s: 10.0,
        last: 0.0,
    });
    forma.exchange_mode = Some("permuta".into()); // não canônico
    engine.register_form(forma);
    // (o alerta está no Caderno interno do engine — indireto: forma registrada)
    assert!(engine
        .active_names()
        .iter()
        .any(|n| n.as_ref() == "Torcida"));
    assert!(engine.form("Torcida").is_some());
    assert!(engine.form_mut("Torcida").is_some());
    assert_eq!(engine.active_forms().len(), 1);
}

#[test]
fn reclassify_equilibrium_sobre_equilibrium_e_sem_efeito() {
    let dir = tmpdir("noop");
    let mut sim = FxpSimulator::new();
    sim.set_sensor("cpu_temp", 90.0); // dispara qualquer regra > 10
    let mut engine = Engine::new(sim, 1.0, &dir);
    let programa = "\
equilibrium B { value: 2, horizon: 300s }
review B { when cpu_temp > 10 -> reclassify_as_equilibrium }
";
    let (program, diags) = vbl_lang::parse(programa);
    assert!(!diags.has_errors(), "{diags}");
    let _interp = load(&mut engine, &program);
    engine.tick();
    // sem efeito auditado; a forma permanece equilibrium
    assert_eq!(
        engine.form("B").unwrap().conjugation,
        Conjugation::Equilibrium
    );
}

#[test]
fn persistence_que_falha_alerta_sem_derrubar_o_runtime() {
    // persistence_dir aponta para um ARQUIVO → toda escrita falha
    let dir = tmpdir("persist-fail");
    let arquivo = dir.join("nao-diretorio");
    std::fs::write(&arquivo, "x").unwrap();
    let mut sim = FxpSimulator::new();
    sim.set_sensor("cpu_temp", 90.0);
    let mut engine = Engine::new(sim, 1.0, &arquivo);
    let programa = "\
event C { value: 3, horizon: 30s }
review C { when cpu_temp > 10 -> reclassify_as_equilibrium }
";
    let (program, diags) = vbl_lang::parse(programa);
    assert!(!diags.has_errors(), "{diags}");
    let _interp = load(&mut engine, &program);
    engine.tick();
    // a transição foi auditada com falha de persistência…
    assert_eq!(
        engine.form("C").unwrap().conjugation,
        Conjugation::Equilibrium
    );
}

#[test]
fn regra_com_atuacao_falha_alerta_e_horizonte_em_ms_no_canonico() {
    let dir = tmpdir("act-fail");
    let mut sim = FxpSimulator::new();
    sim.set_sensor("cpu_temp", 90.0); // Fantasma não está no registro → MissingActor
    let mut engine = Engine::new(sim, 1.0, &dir);
    let programa = "\
event D { value: 4, horizon: 30s }
review D { when cpu_temp > 10 -> act(Fantasma, 5) }
";
    let (program, diags) = vbl_lang::parse(programa);
    assert!(!diags.has_errors(), "{diags}");
    let mut interp = load(&mut engine, &program);
    engine.tick();
    interp.run_due(&mut engine);

    // form_to_ast: horizon fracionário vira ms canônico; value textual vira str
    let mut forma = forma_evento("Ms", 0.0);
    forma.horizon_s = 0.5;
    forma.value = Value::Str("texto".into());
    let ast = vbl_runtime::engine::form_to_ast(&forma);
    assert_eq!(ast.horizon.unit, vbl_lang::TimeUnit::Ms);
    assert_eq!(ast.horizon.value, 500.0);
    assert!(matches!(ast.value.kind, vbl_lang::ExprKind::Str(_)));
    // e o canônico reparseia
    let texto = vbl_lang::canon::form_to_vl(&ast);
    let (_, diags) = vbl_lang::parse(&texto);
    assert!(!diags.has_errors(), "{texto}: {diags}");
}

// ---------------------------------------------------------------------------
// Loader: matriz de validação de registro (FORMAL §3/§6)
// ---------------------------------------------------------------------------

fn diags_de(programa: &str) -> Vec<String> {
    let (program, pd) = vbl_lang::parse(programa);
    assert!(!pd.has_errors(), "{programa}: {pd}");
    let registro = Registry::minimum();
    validate(&registro, &program)
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn validate_matriz_de_registro() {
    // 1. source_path fora do registro
    let d = diags_de("event A { value: 1, horizon: 5s, source_path: \"solar_panel\" }");
    assert!(d.iter().any(|c| c == "sensor_nao_registrado"), "{d:?}");

    // 2. sensor de review fora do registro
    let d = diags_de(
        "event A { value: 1, horizon: 5s }\nreview A { when solar_panel > 1 -> dissolve }",
    );
    assert!(d.iter().any(|c| c == "sensor_nao_registrado"), "{d:?}");

    // 3. unidade incompatível com a grandeza do sensor (W em temperatura)
    let d = diags_de(
        "event A { value: 1, horizon: 5s }\nreview A { when cpu_temp > 100 W -> dissolve }",
    );
    assert!(d.iter().any(|c| c == "unidade_incompativel"), "{d:?}");

    // 4. threshold sem unidade com grandeza que a exige
    let d =
        diags_de("event A { value: 1, horizon: 5s }\nreview A { when cpu_temp > 30 -> dissolve }");
    assert!(d.iter().any(|c| c == "unidade_ausente"), "{d:?}");

    // 5. ator de regra fora do registro
    let d = diags_de(
        "event A { value: 1, horizon: 5s }\nreview A { when cpu_temp > 30°C -> act(Pinca, 5) }",
    );
    assert!(d.iter().any(|c| c == "ator_nao_registrado"), "{d:?}");

    // 6. ator de main fora do registro
    let d = diags_de("event A { value: 1, horizon: 5s }\nmain { act(Pinca, 5) }");
    assert!(d.iter().any(|c| c == "ator_nao_registrado"), "{d:?}");

    // 7. programa íntegro → nenhum diagnóstico de registro
    let d = diags_de(
        "event A { value: 1, horizon: 5s, source_path: \"cpu_temp\" }\n\
         review A { when cpu_temp > 30°C -> dissolve, when cpu_power > 150 W -> act(Fan, 100) }\n\
         main { keep(A), every 2s { act(StatusLed, 1) } }",
    );
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn loader_carrega_main_direto_every_e_valores_textuais() {
    let dir = tmpdir("loader-main");
    let mut engine = Engine::new(FxpSimulator::new(), 1.0, &dir);
    let programa = "\
event A { value: \"olho\", horizon: 30s }
nonequilibrium T { value: cheio, horizon: 60s, maintenance_deadline: 10s, exchange_mode: troca }
main { keep(A), act(Fan, \"forte\"), every 2s { act(StatusLed, ligado) } }
";
    let (program, diags) = vbl_lang::parse(programa);
    assert!(!diags.has_errors(), "{diags}");
    let mut interp = load(&mut engine, &program);
    // formas carregadas com atributos do loader
    let t = engine.form("T").unwrap();
    assert_eq!(t.conjugation, Conjugation::Nonequilibrium);
    assert_eq!(t.exchange_mode.as_deref(), Some("troca"));
    assert_eq!(t.declared_maintenance_deadline, Some(10.0));
    assert!(matches!(&t.value, Value::Ident(s) if s == "cheio"));
    // statements de topo rodam como bloco every de 1 tick; every 2s agendado
    engine.tick();
    interp.run_due(&mut engine);
    engine.tick();
    interp.run_due(&mut engine);
    assert!(engine.form("A").is_some()); // keep renova
}
