//! Testes de transição — reancoragem Rust da suíte da Etapa 1
//! (`tests/unit/test_finitude.py`, `test_tick.py`, `test_atores.py`,
//! `test_falha_sensores.py` e as cláusulas de runtime de
//! `test_clausulas_erro.py`).
//!
//! Os programas são construídos como TEXTO `.vl` e passam pelo parser real
//! da Etapa 2 (`vbl-lang`) — o caminho de produção completo:
//! texto → AST → loader → engine.

use vbl_runtime::ledger::kinds;
use vbl_runtime::form::{RETENTION_BUDGET, CANONICAL_POETIC_VALUE};
use vbl_runtime::json::Json;
use vbl_runtime::scheduler::Deadline;
use vbl_runtime::{
    load, ActorLimits, Engine, FxpSimulator, MainInterpreter,
};
use vbl_lang::parse;

type Eng = Engine<FxpSimulator>;

/// Banco de teste: engine + FXP simulado + interpretador do main.
struct Bank {
    engine: Eng,
    interp: Option<MainInterpreter>,
}

fn default_sim() -> FxpSimulator {
    FxpSimulator::new()
}

/// Parse + carga no engine. `persist` = diretório de persistência.
fn build_with(fxp: FxpSimulator, source: &str, persist: &std::path::Path) -> Bank {
    let (program, diags) = parse(source);
    assert!(!diags.has_errors(), "programa de teste inválido:\n{diags}\n{source}");
    let mut engine = Engine::new(fxp, 1.0, persist);
    let interp = load(&mut engine, &program);
    let interp = if program.main.is_some() { Some(interp) } else { None };
    Bank { engine, interp }
}

fn build_at(fxp: FxpSimulator, source: &str, dir: &std::path::Path) -> Bank {
    let mut b = build_with(fxp, source, dir);
    // recarga de equilibria persistidos (FORMAL §4.1 — na inicialização)
    if std::fs::read_dir(dir).is_ok() {
        vbl_runtime::persist::reload_equilibrium(&mut b.engine);
    }
    b
}

fn assemble(fxp: FxpSimulator, source: &str) -> Bank {
    let dir = std::env::temp_dir().join(format!(
        "vbl-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    build_at(fxp, source, &dir)
}

impl Bank {
    fn tick(&mut self) {
        if let Some(interp) = &mut self.interp {
            interp.run_due(&mut self.engine);
        }
        self.engine.tick();
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            self.tick();
        }
    }

    fn set_sensor(&mut self, name: &str, value: f64) {
        self.engine.fxp.set_sensor(name, value);
    }

    fn has(&self, kind: &str) -> bool {
        self.engine.ledger.has(kind)
    }

    fn count_with(&self, kind: &str, filter: &[(&str, Json)]) -> bool {
        self.engine.ledger.count_with(kind, filter)
    }

    fn count(&self, kind: &str) -> usize {
        self.engine.ledger.kinds().iter().filter(|k| **k == kind).count()
    }

    fn living_form(&self, name: &str) -> bool {
        self.engine.form(name).is_some()
    }
}

impl Drop for Bank {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.engine.persistence_dir());
    }
}

// ======================================================================
// Finitude (test_finitude.py) — FORMAL §4.1
// ======================================================================
#[test]
fn event_expires_at_horizon_with_typed_end() {
    let mut b = assemble(default_sim(), "event Piscada { value: \"impulso_curto\", horizon: 3s }");
    b.tick(); // t=1
    b.tick(); // t=2 — ainda ativa
    assert!(b.living_form("Piscada"));
    b.tick(); // t=3 — horizon esgota (>=)
    assert!(!b.living_form("Piscada"));
    assert!(b.has(kinds::DISSOLVE_HORIZON));
}

#[test]
fn horizon_is_absolute_reclassification_does_not_renew() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium Pensar { value: \"ideia\", horizon: 5s, source_path: \"attention\", maintenance_deadline: 3s }\n\
         review Pensar { when attention < 30% -> reclassify_as_equilibrium }",
    );
    b.set_sensor("attention", 15.0);
    b.tick(); // t=1: reclassifica (persistida)
    assert!(b.count_with(
        kinds::TRANSITION,
        &[("forma", Json::str("Pensar")), ("para", Json::str("equilibrium"))]
    ));
    assert_eq!(b.engine.form("Pensar").unwrap().creation_time, 0.0); // não renovado
    assert!(b.has(kinds::PERSISTENCE)); // gravação `.vl` canônico
    b.set_sensor("attention", 90.0); // regra para de disparar
    b.ticks(3); // t=2,3,4
    assert!(b.living_form("Pensar"));
    b.tick(); // t=5: horizon original esgota
    assert!(!b.living_form("Pensar"));
    assert!(b.has(kinds::DISSOLVE_HORIZON));
}

#[test]
fn equilibrium_also_expires_by_horizon() {
    let mut b = assemble(default_sim(), "equilibrium Registro { value: \"doc\", horizon: 2s }");
    b.tick();
    assert!(b.living_form("Registro")); // t=1: 1>=2? não
    b.tick(); // t=2: limite exato expira
    assert!(!b.living_form("Registro"));
    assert!(b.has(kinds::DISSOLVE_HORIZON));
}

#[test]
fn nonequilibrium_with_active_rule_has_implicit_maintenance() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium Vigilia { value: \"trabalho\", horizon: 30s, source_path: \"attention\", maintenance_deadline: 2s }\n\
         review Vigilia { when attention < 5% -> dissolve }",
    );
    b.ticks(6); // attention = 100 → regra nunca dispara
    assert!(b.living_form("Vigilia"));
    assert!(!b.has(kinds::COLLAPSE_MAINTENANCE));
}

#[test]
fn nonequilibrium_without_maintenance_collapses_on_first_due() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium Solo { value: \"sem_vigilia\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 2s }",
    );
    b.tick(); // t=1: 1 > 2? não
    b.tick(); // t=2: 2 > 2? não (limite estrito)
    assert!(b.living_form("Solo"));
    b.tick(); // t=3: 3 > 2? sim — colapso
    assert!(!b.living_form("Solo"));
    assert!(b.count_with(kinds::COLLAPSE_MAINTENANCE, &[("forma", Json::str("Solo"))]));
}

#[test]
fn manual_keep_renews_deadline() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium Solo { value: \"mantido\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 2s }\n\
         main { every 1s { keep(Solo) } }",
    );
    b.ticks(6);
    assert!(b.living_form("Solo"));
    assert!(!b.has(kinds::COLLAPSE_MAINTENANCE));
}

#[test]
fn form_terminates_once_per_tick() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium Ciclo { value: \"lucro\", horizon: 3s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
         review Ciclo { when cpu_temp > 85°C -> dissolve }",
    );
    b.set_sensor("cpu_temp", 90.0);
    b.tick(); // t=3 não chegou; regra dispara primeiro
    assert!(!b.living_form("Ciclo"));
    assert_eq!(b.count(kinds::DISSOLVE_RULE), 1);
    assert!(!b.has(kinds::DISSOLVE_HORIZON));
}

#[test]
fn retention_counters_within_budgets() {
    let mut b = assemble(
        default_sim(),
        "event Ev { value: \"curto\", horizon: 3s }\n\
         equilibrium Eq { value: \"doc\", horizon: 30s, cost_bytes: 128 }\n\
         nonequilibrium Neq { value: \"trabalho\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 2s }",
    );
    let (event_budget, eq_budget, neq_budget) = RETENTION_BUDGET;
    assert!(b.engine.retention.per_form["Ev"] <= event_budget);
    assert!(b.engine.retention.per_form["Eq"] <= eq_budget);
    assert!(b.engine.retention.per_form["Neq"] <= neq_budget);
    assert!(b.engine.retention.labor["Neq"] > 0);
    b.engine.dissolve_form("Neq", kinds::DISSOLVE_RULE);
    assert!(!b.engine.retention.per_form.contains_key("Neq"));
    assert!(!b.engine.retention.labor.contains_key("Neq")); // 0 bytes retidos
}

// ======================================================================
// Ordem e precedência no tick (test_tick.py) — FORMAL §4.2/§4.5
// ======================================================================
#[test]
fn rules_are_evaluated_in_declared_order() {
    let mut b = assemble(
        default_sim(),
        "event Dupla { value: \"v\", horizon: 30s }\n\
         review Dupla { when cpu_temp > 10°C -> act(Ventoinha, 100),\n\
                        when cpu_temp > 10°C -> act(Ventoinha, 150) }",
    );
    b.set_sensor("cpu_temp", 40.0);
    b.tick();
    let deliveries: Vec<f64> = b
        .engine
        .fxp
        .delivered
        .iter()
        .filter(|m| m.actor == "Ventoinha")
        .filter_map(|m| m.value.as_num())
        .collect();
    assert_eq!(deliveries, vec![100.0, 150.0]); // ordem declarada preservada
}

#[test]
fn review_short_circuit_after_dissolution() {
    let mut b = assemble(
        default_sim(),
        "event Curto { value: \"v\", horizon: 30s }\n\
         review Curto { when cpu_temp > 10°C -> act(Ventoinha, 100), dissolve,\n\
                        when cpu_temp > 10°C -> act(LedIndicador, \"vermelho\") }",
    );
    b.set_sensor("cpu_temp", 40.0);
    b.tick();
    assert!(b.count_with(
        kinds::REVIEW_SHORT_CIRCUIT,
        &[("forma", Json::str("Curto")), ("regras_restantes", Json::num(1.0))]
    ));
    assert!(b.has(kinds::DISSOLVE_RULE));
    assert!(b.engine.fxp.delivered.iter().any(|m| m.actor == "Ventoinha"));
    assert!(!b.engine.fxp.delivered.iter().any(|m| m.actor == "LedIndicador"));
}

#[test]
fn subvert_does_not_cancel_same_rule_act() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium Trading { value: \"lucro\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
         review Trading { when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50) }",
    );
    b.set_sensor("cpu_temp", 86.5);
    b.tick();
    assert!(b.count_with(kinds::DISSOLVE_SUBVERT, &[("forma", Json::str("Trading"))]));
    assert!(b.count_with(
        kinds::SUBVERT_APPLIED,
        &[("novo_valor", Json::str(CANONICAL_POETIC_VALUE))]
    ));
    assert!(b
        .engine
        .fxp
        .delivered
        .iter()
        .any(|m| m.actor == "CpuPowerCap" && m.value == vbl_runtime::Value::Num(50.0)));
    assert!(!b.living_form("Trading")); // dissolvida no mesmo tick
    assert_eq!(b.engine.clock, 1); // ≤ 1 tick virtual
}

#[test]
fn dispatched_actuation_not_revoked_by_subvert() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium T { value: \"v\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
         review T { when cpu_temp > 85°C -> subvert, act(LedIndicador, \"verde\") }",
    );
    b.set_sensor("cpu_temp", 90.0);
    b.tick();
    assert_eq!(
        b.engine.fxp.current_actor("LedIndicador"),
        Some(&vbl_runtime::Value::Str("verde".into()))
    );
}

#[test]
fn notify_shutdown_neither_dissolves_nor_interrupts() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium T { value: \"v\", horizon: 30s, source_path: \"attention\", maintenance_deadline: 10s }\n\
         review T { when attention < 20% -> notify_shutdown, act(LedIndicador, \"apagado\") }",
    );
    b.set_sensor("attention", 10.0);
    b.tick();
    assert!(b.living_form("T")); // forma permanece ativa
    assert!(!b.has(kinds::DISSOLVE_RULE));
    assert_eq!(
        b.engine.fxp.current_actor("LedIndicador"),
        Some(&vbl_runtime::Value::Str("apagado".into()))
    );
}

#[test]
fn equal_sharing_of_global_power() {
    let mut b = assemble(
        default_sim(),
        "event A { value: \"a\", horizon: 30s }\nevent B { value: \"b\", horizon: 30s }",
    );
    b.engine.fxp.set_sensor("cpu_power", 100.0);
    b.tick();
    let leaks = b.engine.ledger.search("LEAK", &[]);
    let per_form: std::collections::BTreeMap<String, f64> = leaks
        .iter()
        .filter_map(|e| match &e.extra {
            Json::Obj(c) => Some((
                match c.get("forma") {
                    Some(Json::Str(s)) => s.clone(),
                    _ => String::new(),
                },
                match c.get("watts") {
                    Some(Json::Num(n)) => *n,
                    _ => 0.0,
                },
            )),
            _ => None,
        })
        .collect();
    assert!((per_form["A"] - 50.0).abs() < 1e-9);
    assert!((per_form["B"] - 50.0).abs() < 1e-9);
    assert!((per_form["A"] + per_form["B"] - 100.0).abs() < 1e-9);
}

#[test]
fn sha256_chain_detects_tampering() {
    let mut b = assemble(default_sim(), "event X { value: \"v\", horizon: 3s }");
    b.tick();
    assert!(b.engine.ledger.verify_chain());
    // adulteração retroativa quebra a cadeia
    b.engine.ledger.events[0].msg = "forjado".into();
    assert!(!b.engine.ledger.verify_chain());
}

#[test]
fn jsonl_export_reproduces_chain() {
    let mut b = assemble(default_sim(), "event X { value: \"v\", horizon: 3s }");
    b.tick();
    std::fs::create_dir_all(b.engine.persistence_dir()).unwrap();
    let path = b.engine.persistence_dir().join("caderno.jsonl");
    let n = b.engine.ledger.export_jsonl(&path).unwrap();
    assert!(n > 0);
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), n);
    // cada linha traz seq/kind/msg/hash — auditoria externa possível
    for (i, line) in lines.iter().enumerate() {
        assert!(line.contains(&format!("\"seq\":{i}")), "linha {i}: {line}");
        assert!(line.contains("\"hash\":\""), "linha {i} sem hash");
    }
    // kinds na ordem
    let first_kind = lines[0].split("\"kind\":\"").nth(1).unwrap().split('"').next().unwrap();
    assert_eq!(first_kind, "INFO"); // "Forma X conjugada..."
}

// ======================================================================
// Atores (test_atores.py) — FORMAL §4.3
// ======================================================================
#[test]
fn act_is_serialized_and_delivered_to_correct_actor() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium Servidor { value: \"critico\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
         review Servidor { when cpu_temp > 70°C -> act(Ventoinha, 200) }",
    );
    b.set_sensor("cpu_temp", 75.0);
    b.tick();
    // mensagem serializada no outbox do FXP
    let msg = b
        .engine
        .fxp
        .outbox
        .iter()
        .find(|m| m.op == "act" && m.actor == "Ventoinha" && m.value == vbl_runtime::Value::Num(200.0))
        .expect("mensagem FXP `act` não serializada");
    assert_eq!(msg.tick, b.engine.clock); // tick de despacho registrado
    assert!(b
        .engine
        .fxp
        .delivered
        .iter()
        .any(|m| m.actor == "Ventoinha" && m.value == vbl_runtime::Value::Num(200.0)));
    assert_eq!(b.engine.fxp.current_actor("Ventoinha"), Some(&vbl_runtime::Value::Num(200.0)));
    assert!(b.count_with(
        "ACTUATION",
        &[("ator", Json::str("Ventoinha")), ("valor", Json::num(200.0)), ("sucesso", Json::boolean(true))]
    ));
}

#[test]
fn nonexistent_actor_rejected_with_registry() {
    let mut b = assemble(
        default_sim(),
        "event Tarefa { value: \"x\", horizon: 10s }\n\
         review Tarefa { when cpu_temp > 10°C -> act(AtorFantasma, 10) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    assert!(b.count_with(kinds::ACTOR_UNKNOWN, &[("ator", Json::str("AtorFantasma"))]));
    assert!(b.count_with("ACTUATION", &[("ator", Json::str("AtorFantasma")), ("sucesso", Json::boolean(false))]));
    assert!(!b.engine.fxp.delivered.iter().any(|m| m.actor == "AtorFantasma"));
}

#[test]
fn value_below_minimum_rejected_without_send() {
    let mut b = assemble(
        default_sim(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(CpuPowerCap, 5) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    let events = b.engine.ledger.search(
        kinds::ACTOR_REJECTED_VALUE,
        &[("ator", Json::str("CpuPowerCap"))],
    );
    assert_eq!(events.len(), 1);
    match &events[0].extra {
        Json::Obj(c) => {
            assert_eq!(c.get("limite"), Some(&Json::str("min")));
            assert_eq!(c.get("limite_valor"), Some(&Json::num(10.0)));
        }
        _ => panic!("extra ausente"),
    }
    assert!(!b.engine.fxp.delivered.iter().any(|m| m.actor == "CpuPowerCap"));
}

#[test]
fn value_above_safety_limit_rejected() {
    let mut b = assemble(
        default_sim(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(Ventoinha, 250) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    let events = b.engine.ledger.search(
        kinds::ACTOR_REJECTED_VALUE,
        &[("ator", Json::str("Ventoinha"))],
    );
    assert_eq!(events.len(), 1);
    match &events[0].extra {
        Json::Obj(c) => {
            assert_eq!(c.get("limite"), Some(&Json::str("safety_limit")));
            assert_eq!(c.get("limite_valor"), Some(&Json::num(200.0)));
        }
        _ => panic!("extra ausente"),
    }
    assert!(!b.engine.fxp.delivered.iter().any(|m| m.actor == "Ventoinha"));
}

#[test]
fn limits_are_inclusive() {
    let mut b = assemble(
        default_sim(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(Ventoinha, 200), act(CpuPowerCap, 10) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    assert_eq!(b.engine.fxp.current_actor("Ventoinha"), Some(&vbl_runtime::Value::Num(200.0)));
    assert_eq!(b.engine.fxp.current_actor("CpuPowerCap"), Some(&vbl_runtime::Value::Num(10.0)));
    assert!(!b.has(kinds::ACTOR_REJECTED_VALUE));
}

#[test]
fn actor_beyond_maximum_rejected_without_send() {
    let mut b = assemble(
        default_sim(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(Ventoinha, 256) }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    assert!(b.has(kinds::ACTOR_REJECTED_VALUE));
    assert!(!b.engine.fxp.delivered.iter().any(|m| m.actor == "Ventoinha"));
}

#[test]
fn act_with_textual_value_without_numeric_limits() {
    let mut b = assemble(
        default_sim(),
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 10°C -> act(LedIndicador, \"verde\") }",
    );
    b.set_sensor("cpu_temp", 30.0);
    b.tick();
    assert_eq!(
        b.engine.fxp.current_actor("LedIndicador"),
        Some(&vbl_runtime::Value::Str("verde".into()))
    );
}

#[test]
fn registry_fallback_triggers_when_primary_fails() {
    let mut fxp = FxpSimulator::new();
    fxp.register_actor("VentoinhaReserva", ActorLimits { min: Some(0.0), max: Some(255.0), safety_limit: Some(200.0) });
    fxp.set_fallback("Ventoinha", &["VentoinhaReserva"]);
    fxp.fail_actor("Ventoinha");
    let mut b = assemble(
        fxp,
        "event T { value: \"x\", horizon: 10s }\n\
         review T { when cpu_temp > 70°C -> act(Ventoinha, 200) }",
    );
    b.set_sensor("cpu_temp", 75.0);
    b.tick();
    // tentativa primária registrada, falha registrada, fallback executado
    assert!(b.count_with("ACTUATION", &[("ator", Json::str("Ventoinha")), ("sucesso", Json::boolean(false))]));
    assert!(b.count_with(kinds::ACTOR_UNAVAILABLE, &[("ator", Json::str("Ventoinha"))]));
    assert!(b.count_with(
        kinds::FALLBACK_EXECUTED,
        &[("primario", Json::str("Ventoinha")), ("alternativo", Json::str("VentoinhaReserva"))]
    ));
    assert_eq!(
        b.engine.fxp.current_actor("VentoinhaReserva"),
        Some(&vbl_runtime::Value::Num(200.0))
    );
    assert!(b
        .engine
        .fxp
        .delivered
        .iter()
        .any(|m| m.actor == "VentoinhaReserva" && m.fallback_of.as_deref() == Some("Ventoinha")));
}

// ======================================================================
// Falha de sensores (test_falha_sensores.py) — FORMAL §4.7
// ======================================================================
const SENTINEL: &str = "\
nonequilibrium Sentinela { value: \"vigia\", horizon: 30s, source_path: \"attention\", maintenance_deadline: 3s }\n\
review Sentinela { when attention < 30% -> reclassify_as_equilibrium }";

#[test]
fn zero_read_is_valid_and_fires_rules() {
    let mut b = assemble(default_sim(), SENTINEL);
    b.set_sensor("attention", 0.0);
    b.tick();
    assert!(b.count_with(
        kinds::TRANSITION,
        &[("forma", Json::str("Sentinela")), ("para", Json::str("equilibrium"))]
    ));
    // zero NÃO é falha de I/O: nenhum alerta de sensor
    assert!(!b.count_with("ALERT", &[("motivo", Json::str("sensor_not_registered"))]));
    assert!(!b.count_with("ALERT", &[("motivo", Json::str("sensor_inaccessible"))]));
}

#[test]
fn missing_sensor_evaluates_no_condition_nor_fires() {
    let mut b = assemble(
        default_sim(),
        "nonequilibrium Fantasma { value: \"obs\", horizon: 30s, source_path: \"sensor_inexistente\", maintenance_deadline: 3s }\n\
         review Fantasma { when sensor_inexistente < 30% -> reclassify_as_equilibrium }",
    );
    b.ticks(4);
    // a regra jamais avaliou: forma permanece nonequilibrium
    assert_eq!(
        b.engine.form("Fantasma").unwrap().conjugation,
        vbl_lang::Conjugation::Nonequilibrium
    );
    assert!(!b.has(kinds::TRANSITION));
    assert!(b.count_with(
        "ALERT",
        &[("motivo", Json::str("sensor_not_registered")), ("sensor", Json::str("sensor_inexistente"))]
    ));
}

#[test]
fn missing_sensor_is_not_treated_as_zero() {
    let mut b = assemble(default_sim(), SENTINEL);
    b.engine.fxp.unregister_sensor("attention");
    b.ticks(3);
    assert!(!b.has(kinds::TRANSITION)); // nenhum disparo falso
    assert_eq!(
        b.engine.form("Sentinela").unwrap().conjugation,
        vbl_lang::Conjugation::Nonequilibrium
    );
}

#[test]
fn registered_inaccessible_sensor_follows_same_rule() {
    let mut b = assemble(default_sim(), SENTINEL);
    b.engine.fxp.fail_sensor("attention");
    b.tick();
    assert!(!b.has(kinds::TRANSITION));
    assert!(b.count_with(
        "ALERT",
        &[("motivo", Json::str("sensor_inaccessible")), ("sensor", Json::str("attention"))]
    ));
    // recupera a acessibilidade e a regra volta a avaliar
    b.engine.fxp.recover_sensor("attention");
    b.set_sensor("attention", 15.0);
    b.tick();
    assert!(b.count_with(
        kinds::TRANSITION,
        &[("forma", Json::str("Sentinela")), ("para", Json::str("equilibrium"))]
    ));
}

#[test]
fn form_without_source_path_generates_no_read_nor_failure() {
    let mut b = assemble(default_sim(), "event Piscada { value: \"impulso_curto\", horizon: 2s }");
    b.tick();
    assert!(b.living_form("Piscada")); // sem crash, sem leitura
    assert!(!b.count_with("ALERT", &[("motivo", Json::str("sensor_not_registered"))]));
}

// ======================================================================
// Cláusulas de erro em runtime (test_clausulas_erro.py, camada runtime)
// ======================================================================
#[test]
fn reclassify_to_nonequilibrium_without_deadline_is_recorded_error() {
    let mut b = assemble(
        default_sim(),
        "event Ev { value: \"conteudo\", horizon: 30s }\n\
         review Ev { when cpu_temp > 90°C -> reclassify_as_nonequilibrium }",
    );
    b.set_sensor("cpu_temp", 95.0);
    b.tick();
    assert!(b.has(kinds::RECLASSIFY_NO_DEADLINE));
    assert_eq!(
        b.engine.form("Ev").unwrap().conjugation,
        vbl_lang::Conjugation::Event
    );
}

#[test]
fn reclassify_nonequilibrium_preserves_declared_deadline() {
    // NEQ -> EQ -> NEQ revive com o deadline DECLARADO original (FORMAL §3)
    let mut b = assemble(
        default_sim(),
        "nonequilibrium P { value: \"v\", horizon: 60s, source_path: \"attention\", maintenance_deadline: 3s, exchange_mode: \"extraction\" }\n\
         review P { when attention < 30% -> reclassify_as_equilibrium,\n\
                    when attention > 80% -> reclassify_as_nonequilibrium }",
    );
    b.set_sensor("attention", 15.0);
    b.tick(); // NEQ -> EQ
    b.set_sensor("attention", 90.0);
    b.tick(); // EQ -> NEQ (deadline 3s declarado preservado)
    assert!(b.count_with(
        kinds::TRANSITION,
        &[("forma", Json::str("P")), ("para", Json::str("nonequilibrium"))]
    ));
    let form = b.engine.form("P").unwrap();
    assert_eq!(form.conjugation, vbl_lang::Conjugation::Nonequilibrium);
    assert_eq!(form.maintenance.as_ref().unwrap().deadline_s, 3.0);
}

// ======================================================================
// Persistência (FORMAL §4.1) — `.vl` canônico + recarga
// ======================================================================
#[test]
fn reclassify_persists_reparseable_canonical_vl_with_sha256() {
    let dir = std::env::temp_dir().join(format!(
        "vbl-persist-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let sha;
    let persisted_text;
    {
        let mut b = build_at(
            default_sim(),
            "nonequilibrium Pensar { value: \"ideia\", horizon: 60s, source_path: \"attention\", maintenance_deadline: 3s, exchange_mode: \"cooperation\" }\n\
             review Pensar { when attention < 30% -> reclassify_as_equilibrium }",
            &dir,
        );
        b.set_sensor("attention", 15.0);
        b.tick();
        sha = b
            .engine
            .ledger
            .search(kinds::PERSISTENCE, &[("forma", Json::str("Pensar"))])
            .iter()
            .map(|e| match &e.extra {
                Json::Obj(c) => match c.get("sha256") {
                    Some(Json::Str(s)) => s.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            })
            .next()
            .unwrap();
        assert_eq!(sha.len(), 64);
        assert!(b.living_form("Pensar"));
        persisted_text = std::fs::read_to_string(dir.join("Pensar.vl")).unwrap();
    }
    // o arquivo gravado é reparseável e reproduz o SHA registrado
    let text = persisted_text;
    let (_, diags) = vbl_lang::parse(&text);
    assert!(!diags.has_errors(), "persistido não reparseou: {diags}\n{text}");
    assert_eq!(vbl_runtime::ledger::sha256_hex(text.as_bytes()), sha);
    // a forma persistida é a pós-transição: equilibrium (FORMAL §4.1),
    // com source_path preservado e horizon absoluto (60s, não renovado)
    assert!(text.contains("equilibrium Pensar"), "{text}");
    assert!(text.contains("source_path: \"attention\""), "{text}");
    assert!(text.contains("horizon: 60s"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_reloads_equilibrium_with_unexpired_horizon() {
    let dir = std::env::temp_dir().join(format!(
        "vbl-recarga-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // simula uma persistência anterior (horizon não vencido)
    std::fs::write(
        dir.join("Doc.vl"),
        "equilibrium Doc {\n    value: \"documento\",\n    horizon: 100s,\n    cost_bytes: 64\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("Doc.json"), "{ \"creation_time\": 0 }\n").unwrap();
    // e uma expirada: criada 10s antes da época do programa, horizon 1s
    std::fs::write(
        dir.join("Velho.vl"),
        "equilibrium Velho {\n    value: \"antigo\",\n    horizon: 1s\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("Velho.json"), "{ \"creation_time\": -10 }\n").unwrap();

    let fxp = default_sim();
    let mut engine = Engine::new(fxp, 1.0, &dir);
    let reloaded = vbl_runtime::persist::reload_equilibrium(&mut engine);
    assert_eq!(reloaded, 1);
    assert!(engine.form("Doc").is_some());
    assert!(engine.form("Velho").is_none());
    assert_eq!(engine.form("Doc").unwrap().cost_bytes, Some(64));
    assert!(engine.ledger.count_with("INFO", &[("motivo", Json::str("recarga"))]));
    assert!(engine.ledger.verify_chain());
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Escalonador — invariante de prazos (min-heap)
// ======================================================================
#[test]
fn scheduler_keeps_minheap_by_deadline() {
    let mut s = vbl_runtime::scheduler::Scheduler::new();
    s.schedule("A", Deadline::Horizon, 5.0, 1);
    s.schedule("B", Deadline::Horizon, 2.0, 2);
    s.schedule("C", Deadline::Maintenance, 7.0, 3);
    assert_eq!(s.next().unwrap().form, "B");
    let due = s.drain_due(3.0);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].form, "B");
    assert_eq!(s.next().unwrap().form, "A");
    s.remove_form("A");
    assert_eq!(s.next().unwrap().form, "C");
}

#[test]
fn heap_does_not_grow_when_renewing_maintenance() {
    // keep implícito por 50 ticks: entradas vencidas são consumidas — o
    // tamanho do heap permanece limitado (1 horizon + 1 manutenção + ruído)
    let mut b = assemble(
        default_sim(),
        "nonequilibrium V { value: \"v\", horizon: 1000s, source_path: \"cpu_power\", maintenance_deadline: 2s }\n\
         review V { when cpu_temp > 999°C -> dissolve }",
    );
    b.ticks(50);
    assert!(b.living_form("V"));
    assert!(b.engine.scheduler.len() <= 8, "heap cresceu: {}", b.engine.scheduler.len());
}

// ======================================================================
// Cenário BDD Caso 1 reancorado (fadiga de atenção) — ponta a ponta
// ======================================================================
#[test]
fn case1_attention_fatigue_end_to_end() {
    let dir = std::env::temp_dir().join(format!(
        "vbl-caso1-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    {
        let mut b = build_at(
            default_sim(),
            "nonequilibrium PensarLivre { value: \"consciencia_anteneoliberal_ativa\", horizon: 60s, source_path: \"attention\", maintenance_deadline: 3s, exchange_mode: \"cooperation\" }\n\
             review PensarLivre { when attention < 30% -> reclassify_as_equilibrium }",
            &dir,
        );
        b.set_sensor("attention", 15.0); // atenção esgotada
        b.tick();
        // transição gravada + persistência com SHA-256 + manutenção cessada
        assert!(b.count_with(
            kinds::TRANSITION,
            &[("forma", Json::str("PensarLivre")), ("para", Json::str("equilibrium"))]
        ));
        assert!(b.has(kinds::PERSISTENCE));
        assert_eq!(b.engine.form("PensarLivre").unwrap().maintenance, None);
        assert!(!b.engine.retention.labor.contains_key("PensarLivre")); // 0 bytes de trabalho
        assert!(b.engine.ledger.verify_chain());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Etapa 5 — decisão AD pendente da Etapa 4 (docs/STAGE-4-REPORT.md §7):
// as regras de revisão SOBREVIVEM à reclassificação e permanecem ativas na
// `equilibrium`. Fundamento na FORMAL: o diagrama de estados (§4.1) lista
// `revisão` como caminho de EQ → DIS e a §4.2 manda avaliar as condições de
// revisão para CADA forma ativa — nada as desliga na `equilibrium`. O que
// cessa é a MANUTENÇÃO implícita (a conjugação não recebe ticks de
// manutenção — BDD Caso 1). Disparo sobre forma já na conjugação-alvo é
// no-op AUDITADO (AVALIACAO "já equilibrium"), sem nova transição.
// ======================================================================
#[test]
fn rules_stay_active_in_equilibrium_ad_decision() {
    let mut b = assemble(default_sim(), SENTINEL);
    b.set_sensor("attention", 15.0);
    b.tick(); // regra dispara: NEQ → EQ (persistida)
    assert!(b.count_with(
        kinds::TRANSITION,
        &[("forma", Json::str("Sentinela")), ("para", Json::str("equilibrium"))]
    ));
    assert_eq!(
        b.engine.form("Sentinela").unwrap().conjugation,
        vbl_lang::Conjugation::Equilibrium
    );

    // ticks seguintes: a regra continua avaliada (condição segue verdadeira)
    // e o disparo é no-op auditado — sem segunda transição, sem dissolução
    b.ticks(3);
    assert_eq!(
        b.engine.form("Sentinela").unwrap().conjugation,
        vbl_lang::Conjugation::Equilibrium,
        "a forma permanece equilibrium — no-op não reclassifica nem dissolve"
    );
    assert!(b.count_with(
        "ASSESSMENT",
        &[("forma", Json::str("Sentinela")), ("de", Json::str("equilibrium"))]
    ));

    // a manutenção implícita cessa: sem prazo de manutenção e sem trabalho
    // retido (FORMAL §4.1; BDD Caso 1: "deixa de receber ticks de manutenção")
    assert_eq!(b.engine.form("Sentinela").unwrap().maintenance, None);
    assert!(!b.engine.retention.labor.contains_key("Sentinela"));
}
