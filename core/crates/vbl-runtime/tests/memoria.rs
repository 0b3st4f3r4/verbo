//! Orçamentos de heap — fechamento físico da Etapa 5 (PLAN §5.1; AGENTS
//! §1.3/§2.2). RODA APENAS COM A FEATURE:
//!
//! ```bash
//! cargo test -p vbl-runtime --features heap-audit --test memoria -- --test-threads=1
//! ```
//!
//! IMPORTANTE: o auditor mede a heap do PROCESSO inteiro — os testes têm de
//! rodar SERIAL (`--test-threads=1`), senão as medições se contaminam entre
//! testes paralelos.
//!
//! A ADR-001 usou contadores de retenção como PROXY determinístico de heap;
//! aqui o [`heap_auditor`] (alocador global de contagem) fecha a medição
//! física: heap corrente/pico/total por forma e por conjugação, steady-state
//! de 10.000 formas e churn de 200 mil ciclos de vida — "vazamento inerte"
//! (estrutura em heap além do horizon) quebra os asserts.
//!
//! O Caderno nos testes é [`NoopLedger`]: mede-se o RUNTIME (formas +
//! escalonador + ordem + retenção), não o logger — o custo do Caderno de
//! produção já foi medido à parte na Etapa 4 (≲ 1 MB @ 10k formas).

#![cfg(feature = "heap-audit")]

use vbl_lang::{parse, Conjugation};
use vbl_runtime::ledger::NoopLedger;
use vbl_runtime::form::Form;
use vbl_runtime::fxp::Value;
use vbl_runtime::heap_auditor;
use vbl_runtime::{load, Engine, FxpSimulator};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vbl-memoria-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Forma `event` mínima construída direto no runtime (churn determinístico).
fn event_form(name: &str, now: f64) -> Form {
    Form {
        name: name.into(),
        value: Value::Str("carga".into()),
        horizon_s: 3.0,
        creation_time: now,
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

/// Carrega `n` formas de uma conjugação (com review para nonequilibrium),
/// roda 5 ticks e devolve a heap corrente medida por delta.
fn heap_per_conjugation(n: usize, conjugation: Conjugation) -> (usize, usize) {
    let mut program = String::new();
    for i in 0..n {
        match conjugation {
            Conjugation::Event => {
                program.push_str(&format!("event F{i} {{ value: \"v{i}\", horizon: 1000000s }}\n"));
            }
            Conjugation::Equilibrium => {
                program.push_str(&format!(
                    "equilibrium F{i} {{ value: \"v{i}\", horizon: 1000000s, cost_bytes: 64 }}\n"
                ));
            }
            Conjugation::Nonequilibrium => {
                program.push_str(&format!(
                    "nonequilibrium F{i} {{ value: \"v{i}\", horizon: 1000000s, source_path: \"attention\", maintenance_deadline: 5s }}\n"
                ));
                program.push_str(&format!(
                    "review F{i} {{ when attention < 30% -> reclassify_as_equilibrium }}\n"
                ));
            }
        }
    }
    let (p, d) = parse(&program);
    assert!(!d.has_errors());
    let dir = temp_dir("conj");
    let mut engine = Engine::with_ledger(FxpSimulator::new(), 1.0, &dir, NoopLedger);
    engine.fxp.set_sensor("attention", 50.0);

    heap_auditor::zero();
    load(&mut engine, &p);
    for _ in 0..5 {
        engine.tick();
    }
    let heap = heap_auditor::current().max(0) as usize;
    let peak = heap_auditor::peak().max(0) as usize;
    assert_eq!(engine.active_forms().len(), n, "todas as formas seguem ativas");
    let _ = std::fs::remove_dir_all(&dir);
    (heap, peak)
}

/// Orçamento por forma (AGENTS §1.3): heap por forma `event` / `equilibrium`
/// / `nonequilibrium` (com review). Os limites AGENTS (256 B / 1 KB / 512 B)
/// eram PROVISÓRIOS (AGENTS §4): os medidos incluem os nós dos mapas, chaves
/// e entradas do escalonador — a revisão da meta usa estes números.
#[test]
fn heap_budget_per_form_and_conjugation() {
    let n = 10_000;
    let (heap_ev, _) = heap_per_conjugation(n, Conjugation::Event);
    let (heap_eq, _) = heap_per_conjugation(n, Conjugation::Equilibrium);
    let (heap_neq, _) = heap_per_conjugation(n, Conjugation::Nonequilibrium);

    let per_event = heap_ev / n;
    let per_equil = heap_eq / n;
    let per_neq = heap_neq / n;
    println!("heap/forma event         = {per_event} B (AGENTS provisório: 256 B)");
    println!("heap/forma equilibrium   = {per_equil} B (AGENTS provisório: 1 KB)");
    println!("heap/forma nonequilibrium= {per_neq} B (AGENTS provisório: 512 B)");

    // Ordem relativa esperada (event ≈ equilibrium < nonequilibrium c/ regra)
    assert!(per_event < per_neq, "event deve reter menos que nonequilibrium+review");
    assert!(per_equil < per_neq, "equilibrium deve reter menos que nonequilibrium+review");
    // Tetos REVISTOS da Etapa 5 (AGENTS §4 — meta provisória medida:
    // event/equilibrium 743 B, nonequilibrium+review 1448 B na máquina de
    // referência; detalhes em docs/reports/STAGE-5-REPORT.md §2):
    assert!(per_event <= 1_024, "event: {per_event} B/forma fora do teto revisado (1 KB)");
    assert!(per_equil <= 1_024, "equilibrium: {per_equil} B/forma fora do teto revisado (1 KB)");
    assert!(per_neq <= 2_048, "nonequilibrium+review: {per_neq} B/forma fora do teto revisado (2 KB)");
}

/// Steady-state (PLAN §5 mitigação): 10.000 formas ativas — heap TOTAL do
/// runtime dentro de 10 MB.
#[test]
fn steady_state_10k_forms_within_10mb() {
    let (heap, peak) = heap_per_conjugation(10_000, Conjugation::Event);
    println!("steady-state 10k event: heap = {} B, pico = {} B", heap, peak);
    assert!(heap <= 10 * 1024 * 1024, "steady-state {heap} B > 10 MB");
}

/// "Vazamento inerte" (PLAN §5.1): 200 mil formas nascem e dissolvem pelo
/// caminho NATURAL do runtime (horizon); ao final, com todas dissolvidas, a
/// heap retorna ao baseline — nenhuma estrutura sobrevive ao horizonte.
#[test]
fn churn_200k_forms_heap_returns_to_baseline() {
    let dir = temp_dir("churn");
    let mut engine = Engine::with_ledger(FxpSimulator::new(), 1.0, &dir, NoopLedger);

    // baseline: tick com o sistema vazio
    engine.tick();
    let baseline = {
        heap_auditor::zero();
        engine.tick();
        heap_auditor::current()
    };

    // churn: 200 formas novas por tick, horizonte de 3 ticks dissolve as
    // antigas pelo caminho natural (scheduler → dissolve_horizon)
    let mut n = 0u64;
    for _ in 0..1000 {
        for _ in 0..200 {
            engine.register_form(event_form(&format!("c{n}"), engine.sim_time));
            n += 1;
        }
        engine.tick();
    }
    let peak = heap_auditor::peak();
    // esgota os horizontes restantes: mais 3 ticks sem reposição
    for _ in 0..3 {
        engine.tick();
    }
    assert!(engine.active_forms().is_empty(), "todas as formas devem dissolver");
    let remaining = heap_auditor::current();

    println!("churn: {n} formas, pico de heap = {peak} B");
    println!("baseline = {baseline} B, restante após dissolução = {remaining} B");
    // folga de 256 KB para capacidades retidas (Vec/BTreeMap esvaziados não
    // devolvem capacidade ao alocador — capacidade não é vazamento: não cresce)
    let slack = 256 * 1024isize;
    assert!(
        remaining <= baseline + slack,
        "heap pós-dissolução ({remaining} B) excede baseline+folga ({} B) — inércia oculta!",
        baseline + slack
    );
    assert!(remaining <= peak, "sanidade: restante ≤ pico");
    let _ = std::fs::remove_dir_all(&dir);
}
