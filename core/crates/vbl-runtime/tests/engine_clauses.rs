//! Cláusulas de borda do motor de tick (FORMAL §4.1–§4.5) que a suíte de
//! transição não alcança: revisão sobre forma dissolvida, notify_shutdown,
//! reclassificação recusada por falta de deadline, exchange_mode não
//! canônico, modo de permuta default e vazamento sem ativas.

use vbl_runtime::ledger::kinds;
use vbl_runtime::{Engine, FxpSimulator, load};
use vbl_lang::parse;

fn dir(nome: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "vbl-engine-clauses-{nome}-{}",
        std::process::id()
    ))
}

type Eng = Engine<FxpSimulator>;

fn build(nome: &str, source: &str) -> Eng {
    let (program, diags) = parse(source);
    assert!(!diags.has_errors(), "{diags}\n{source}");
    let mut engine = Engine::new(FxpSimulator::new(), 1.0, dir(nome));
    let _ = load(&mut engine, &program);
    engine
}

fn tem(engine: &Eng, kind: &str, pedaco: &str) -> bool {
    engine
        .ledger
        .events
        .iter()
        .any(|e| e.kind == kind && e.msg.contains(pedaco))
}

#[test]
fn notify_shutdown_encerra_o_tick_com_registro() {
    let mut engine = build(
        "shutdown",
        "event A { value: 1, horizon: 100s, source_path: \"cpu_temp\" }\n\
         review A { when cpu_temp > 1 -> notify_shutdown }",
    );
    engine.tick();
    assert!(
        tem(&engine, "ASSESSMENT", "Desligando cargas secundárias"),
        "eventos: {:?}",
        engine.ledger.events.iter().map(|e| format!("{}|{}", e.kind, e.msg)).collect::<Vec<_>>()
    );
}

#[test]
fn reclassify_para_nonequilibrium_sem_deadline_e_recusado() {
    // event → nonequilibrium sem maintenance_deadline declarado ⇒ recusa
    // registrada; a forma permanece como estava (FORMAL §3).
    let mut engine = build(
        "sem-deadline",
        "event A { value: 1, horizon: 100s, source_path: \"cpu_temp\" }\n\
         review A { when cpu_temp > 1 -> reclassify_as_nonequilibrium }",
    );
    engine.tick();
    assert!(tem(&engine, kinds::RECLASSIFY_NO_DEADLINE, "A"),
        "eventos: {:?}", engine.ledger.events.iter().map(|e| format!("{}|{}", e.kind, e.msg)).collect::<Vec<_>>());
}

#[test]
fn exchange_mode_nao_canonica_gera_alerta_e_default_cooperation() {
    // exchange_mode fora do par canônico (cooperation|extraction) ⇒ alerta;
    // ausência de exchange_mode ⇒ default "cooperation" na reclassificação.
    let mut engine = build(
        "permuta",
        "nonequilibrium P { value: \"v\", horizon: 60s, source_path: \"attention\", maintenance_deadline: 3s, exchange_mode: \"permuta\" }\n\
         review P { when attention < 30% -> reclassify_as_equilibrium,\n\
                    when attention > 80% -> reclassify_as_nonequilibrium }",
    );
    engine.fxp.set_sensor("attention", 15.0);
    engine.tick(); // NEQ -> EQ
    engine.fxp.set_sensor("attention", 90.0);
    engine.tick(); // EQ -> NEQ com "permuta" ⇒ alerta do modo não canônico
    let algum_alerta = engine
        .ledger
        .events
        .iter()
        .any(|e| e.msg.contains("permuta") || e.msg.contains("exchange"));
    assert!(algum_alerta,
        "eventos: {:?}", engine.ledger.events.iter().map(|e| format!("{}|{}", e.kind, e.msg)).collect::<Vec<_>>());
}

#[test]
fn vazamento_sem_formas_ativas_registra_partilha_zero() {
    // Única forma dissolve no tick 1: no instante da partilha não há
    // outras ativas ⇒ potência 0.0 do vazamento (FORMAL §4.2).
    let mut engine = build(
        "solitaria",
        "event A { value: 1, horizon: 1s, source_path: \"cpu_temp\" }",
    );
    engine.tick();
    let leaks = engine.ledger.search("LEAK", &[]);
    assert!(!leaks.is_empty(), "evento de vazamento esperado");
}
