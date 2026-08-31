//! Carregador: AST → runtime (formas, reviews e bloco `main`).
//!
//! Equivalente Rust do `loader.py` + `contract.py` da Etapa 1: carrega o
//! programa no engine e valida referências contra o REGISTRO do FXP
//! (sensor/ator registrados, unidade compatível com a grandeza — FORMAL §3/§6).
//! As cláusulas estruturais já são erros de compilação do parser (`vbl-lang`).
//!
//! `load` NÃO exige registro válido (o runtime lida com falhas de I/O
//! por §4.7 — sensor ausente nunca dispara regra); `validate` devolve os
//! diagnósticos de registro para o `check` do CLI.

use crate::ledger::Ledger;
use crate::engine::Engine;
use crate::form::{ActionRt, Form, Maintenance, RuleRt};
use crate::fxp::{Fxp, Value};
use crate::main_interp::{MainInterpreter, StmtRt};
use vbl_lang::{Action, Conjugation, Declaration, ExprKind, Program, Statement};

/// Diagnóstico de carga (referências contra o registro do FXP).
#[derive(Debug, Clone)]
pub struct LoadDiag {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for LoadDiag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

/// Carrega o programa no engine (formas na ordem de declaração, regras na
/// ordem declarada, bloco `main` interpretado). Devolve o interpretador do
/// `main` (vazio se o programa não tiver `main`).
pub fn load<F: Fxp, C: Ledger>(
    engine: &mut Engine<F, C>,
    program: &Program,
) -> MainInterpreter {
    for decl in &program.decls {
        match decl {
            Declaration::Form(f) => engine.register_form(runtime_form(f, engine.sim_time)),
            Declaration::Review(r) => {
                for rule in &r.rules {
                    let actions: Vec<ActionRt> = rule.actions.iter().map(acao_runtime).collect();
                    let rt = RuleRt {
                        sensor: rule.sensor.name.clone(),
                        op: rule.op,
                        threshold: rule.threshold.value, // unidade vira número puro (FORMAL §3)
                        actions,
                    };
                    engine.form_mut(&r.form).unwrap_or_else(|| {
                        panic!(
                            "review sem forma: '{}' — cláusula de compilação (review_orfa) não verificada",
                            r.form
                        )
                    })
                    .rules
                    .push(rt);
                }
            }
        }
    }
    let mut interp = MainInterpreter::default();
    if let Some(main) = &program.main {
        let mut direct: Vec<StmtRt> = Vec::new();
        for st in &main.statements {
            match statement_runtime(st) {
                StmtKind::Direct(s) => direct.push(s),
                StmtKind::Every(period_s, body) => interp.add_every(period_s, body),
            }
        }
        if !direct.is_empty() {
            // statements de topo rodam como bloco `every` de 1 tick (Etapa 1)
            interp.add_every(engine.tick_seconds(), direct);
        }
    }
    interp
}

enum StmtKind {
    Direct(StmtRt),
    Every(f64, Vec<StmtRt>),
}

fn statement_runtime(st: &Statement) -> StmtKind {
    match st {
        Statement::Keep(form) => StmtKind::Direct(StmtRt::Keep(form.clone())),
        Statement::Act { actor, value } => StmtKind::Direct(StmtRt::Act {
            actor: actor.clone(),
            value: expr_value(value),
        }),
        Statement::Every { period, body } => StmtKind::Every(
            period.seconds(),
            body.iter().map(statement_runtime_direct).collect(),
        ),
    }
}

fn statement_runtime_direct(st: &Statement) -> StmtRt {
    match statement_runtime(st) {
        StmtKind::Direct(s) => s,
        StmtKind::Every(..) => unreachable!("every aninhado tratado como direto"),
    }
}

fn expr_value(expr: &vbl_lang::Expression) -> Value {
    match &expr.kind {
        ExprKind::Str(s) => Value::Str(s.clone()),
        ExprKind::Num(n) => Value::Num(*n),
        ExprKind::Ident(s) => Value::Ident(s.clone()),
    }
}

fn acao_runtime(a: &Action) -> ActionRt {
    match a {
        Action::Dissolve => ActionRt::Dissolve,
        Action::Subvert => ActionRt::Subvert,
        Action::ReclassifyAsEquilibrium => ActionRt::ReclassifyEquilibrium,
        Action::ReclassifyAsNonequilibrium => ActionRt::ReclassifyNonequilibrium,
        Action::NotifyShutdown => ActionRt::NotifyShutdown,
        Action::Act { actor, value, .. } => {
            ActionRt::Act { actor: actor.clone(), value: expr_value(value) }
        }
    }
}

/// Converte a declaração de forma para a forma runtime (criação em `now`).
pub fn runtime_form(f: &vbl_lang::FormDecl, now: f64) -> Form {
    let horizon = f.horizon.seconds();
    let mut form = Form {
        name: f.name.clone(),
        value: expr_value(&f.value),
        horizon_s: horizon,
        creation_time: now,
        conjugation: f.conjugation,
        currency: f
            .attrs
            .currency
            .clone()
            .unwrap_or_else(|| f.conjugation.default_currency().into()),
        source_path: f.attrs.source_path.clone(),
        classification: f.attrs.classification.clone(),
        declared_maintenance_deadline: None,
        maintenance: None,
        exchange_mode: None,
        cost_bytes: None,
        rules: Vec::new(),
        dissolved: false,
        horizon_version: 0,
        maintenance_version: 0,
    };
    match f.conjugation {
        Conjugation::Nonequilibrium => {
            let deadline = f
                .attrs
                .maintenance_deadline
                .as_ref()
                .map(|d| d.seconds())
                .unwrap_or_else(|| {
                    // cláusula de compilação (maintenance_deadline_ausente);
                    // defesa: programa inválido não chega aqui pelo CLI
                    panic!("nonequilibrium '{}' sem maintenance_deadline", f.name)
                });
            form.declared_maintenance_deadline = Some(deadline);
            form.maintenance = Some(Maintenance { deadline_s: deadline, last: now });
            form.exchange_mode =
                Some(f.attrs.exchange_mode.clone().unwrap_or_else(|| "cooperation".into()));
        }
        Conjugation::Equilibrium => {
            form.cost_bytes = f.attrs.cost_bytes.map(|b| b.max(0) as u64);
        }
        Conjugation::Event => {}
    }
    form
}

// ------------------------------------------------------------------
// Validação contra o registro do FXP (contract.py da Etapa 1)
// ------------------------------------------------------------------
/// Valida referências do programa contra o registro do FXP. Devolve apenas
/// diagnósticos de REGISTRO (os estruturais são do parser).
pub fn validate(registry: &crate::fxp::Registry, program: &Program) -> Vec<LoadDiag> {
    let mut diags = Vec::new();
    for f in program.forms() {
        if let Some(sp) = &f.attrs.source_path {
            if !registry.sensores.contains_key(sp) {
                diags.push(LoadDiag {
                    code: "sensor_nao_registrado".into(),
                    message: format!("source_path '{sp}' fora do registro do FXP"),
                });
            }
        }
    }
    for r in program.reviews() {
        for (i, rule) in r.rules.iter().enumerate() {
            let Some(info) = registry.sensores.get(&rule.sensor.name) else {
                diags.push(LoadDiag {
                    code: "sensor_nao_registrado".into(),
                    message: format!(
                        "review {} regra#{}: sensor '{}' fora do registro",
                        r.form, i, rule.sensor.name
                    ),
                });
                continue;
            };
            if let Some(unit) = rule.threshold.unit {
                // unidade compatível com a grandeza do sensor (FORMAL §3)
                if !unit.quantity().is_empty()
                    && !info.quantity.is_empty()
                    && unit.quantity() != info.quantity
                {
                    diags.push(LoadDiag {
                        code: "unidade_incompativel".into(),
                        message: format!(
                            "review {} regra#{}: unidade '{}' incompatível com a grandeza '{}' do sensor '{}' (esperado por grandeza: '{}')",
                            r.form,
                            i,
                            unit.symbol(),
                            info.quantity,
                            rule.sensor.name,
                            info.unit
                        ),
                    });
                }
            } else {
                diags.push(LoadDiag {
                    code: "unidade_ausente".into(),
                    message: format!(
                        "review {} regra#{}: sensor '{}' tem grandeza '{}' — threshold exige unidade ({}), FORMAL §3/§6",
                        r.form, i, rule.sensor.name, info.quantity, info.unit
                    ),
                });
            }
            for action in &rule.actions {
                if let Action::Act { actor, .. } = action {
                    if !registry.actors.contains_key(actor) {
                        diags.push(LoadDiag {
                            code: "ator_nao_registrado".into(),
                            message: format!(
                                "review {} regra#{}: ator '{actor}' fora do registro FXP",
                                r.form, i
                            ),
                        });
                    }
                }
            }
        }
    }
    if let Some(main) = &program.main {
        fn pass(stmts: &[Statement], registry: &crate::fxp::Registry, diags: &mut Vec<LoadDiag>) {
            for st in stmts {
                match st {
                    Statement::Act { actor, .. } => {
                        if !registry.actors.contains_key(actor) {
                            diags.push(LoadDiag {
                                code: "ator_nao_registrado".into(),
                                message: format!("main: ator '{actor}' fora do registro FXP"),
                            });
                        }
                    }
                    Statement::Every { body, .. } => pass(body, registry, diags),
                    Statement::Keep(_) => {}
                }
            }
        }
        pass(&main.statements, registry, &mut diags);
    }
    diags
}
