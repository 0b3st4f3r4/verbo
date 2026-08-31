//! Carregador: AST → runtime (formas, reviews e bloco `main`).
//!
//! Equivalente Rust do `loader.py` + `contract.py` da Etapa 1: carrega o
//! programa no engine e valida referências contra o REGISTRO do FXP
//! (sensor/ator registrados, unidade compatível com a grandeza — FORMAL §3/§6).
//! As cláusulas estruturais já são erros de compilação do parser (`vbl-lang`).
//!
//! `carregar` NÃO exige registro válido (o runtime lida com falhas de I/O
//! por §4.7 — sensor ausente nunca dispara regra); `validar` devolve os
//! diagnósticos de registro para o `check` do CLI.

use crate::notebook::Caderno;
use crate::engine::Engine;
use crate::form::{ActionRt, Form, Manutencao, RuleRt};
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
pub fn carregar<F: Fxp, C: Caderno>(
    engine: &mut Engine<F, C>,
    programa: &Program,
) -> MainInterpreter {
    for decl in &programa.decls {
        match decl {
            Declaration::Form(f) => engine.registrar_forma(forma_runtime(f, engine.sim_time)),
            Declaration::Review(r) => {
                for regra in &r.rules {
                    let acoes: Vec<ActionRt> = regra.actions.iter().map(acao_runtime).collect();
                    let rt = RuleRt {
                        sensor: regra.sensor.nome.clone(),
                        op: regra.op,
                        threshold: regra.threshold.valor, // unidade vira número puro (FORMAL §3)
                        actions: acoes,
                    };
                    engine.forma_mut(&r.form).unwrap_or_else(|| {
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
    if let Some(main) = &programa.main {
        let mut diretos: Vec<StmtRt> = Vec::new();
        for st in &main.statements {
            match statement_runtime(st) {
                StmtKind::Direto(s) => diretos.push(s),
                StmtKind::Every(periodo_s, body) => interp.add_every(periodo_s, body),
            }
        }
        if !diretos.is_empty() {
            // statements de topo rodam como bloco `every` de 1 tick (Etapa 1)
            interp.add_every(engine.tick_seconds(), diretos);
        }
    }
    interp
}

enum StmtKind {
    Direto(StmtRt),
    Every(f64, Vec<StmtRt>),
}

fn statement_runtime(st: &Statement) -> StmtKind {
    match st {
        Statement::Keep(forma) => StmtKind::Direto(StmtRt::Keep(forma.clone())),
        Statement::Act { actor, value } => StmtKind::Direto(StmtRt::Act {
            ator: actor.clone(),
            valor: expr_value(value),
        }),
        Statement::Every { period, body } => StmtKind::Every(
            period.segundos(),
            body.iter().map(statement_runtime_direto).collect(),
        ),
    }
}

fn statement_runtime_direto(st: &Statement) -> StmtRt {
    match statement_runtime(st) {
        StmtKind::Direto(s) => s,
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
            ActionRt::Act { ator: actor.clone(), valor: expr_value(value) }
        }
    }
}

/// Converte a declaração de forma para a forma runtime (criação em `agora`).
pub fn forma_runtime(f: &vbl_lang::FormDecl, agora: f64) -> Form {
    let horizon = f.horizon.segundos();
    let mut form = Form {
        name: f.name.clone(),
        value: expr_value(&f.value),
        horizon_s: horizon,
        creation_time: agora,
        conjugation: f.conjugation,
        currency: f
            .attrs
            .currency
            .clone()
            .unwrap_or_else(|| f.conjugation.currency_padrao().into()),
        source_path: f.attrs.source_path.clone(),
        classification: f.attrs.classification.clone(),
        declared_maintenance_deadline: None,
        manutencao: None,
        exchange_mode: None,
        cost_bytes: None,
        rules: Vec::new(),
        dissolvida: false,
        horizon_versao: 0,
        manutencao_versao: 0,
    };
    match f.conjugation {
        Conjugation::Nonequilibrium => {
            let deadline = f
                .attrs
                .maintenance_deadline
                .as_ref()
                .map(|d| d.segundos())
                .unwrap_or_else(|| {
                    // cláusula de compilação (maintenance_deadline_ausente);
                    // defesa: programa inválido não chega aqui pelo CLI
                    panic!("nonequilibrium '{}' sem maintenance_deadline", f.name)
                });
            form.declared_maintenance_deadline = Some(deadline);
            form.manutencao = Some(Manutencao { deadline_s: deadline, ultima: agora });
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
pub fn validar(registry: &crate::fxp::Registry, programa: &Program) -> Vec<LoadDiag> {
    let mut diags = Vec::new();
    for f in programa.forms() {
        if let Some(sp) = &f.attrs.source_path {
            if !registry.sensores.contains_key(sp) {
                diags.push(LoadDiag {
                    code: "sensor_nao_registrado".into(),
                    message: format!("source_path '{sp}' fora do registro do FXP"),
                });
            }
        }
    }
    for r in programa.reviews() {
        for (i, regra) in r.rules.iter().enumerate() {
            let Some(info) = registry.sensores.get(&regra.sensor.nome) else {
                diags.push(LoadDiag {
                    code: "sensor_nao_registrado".into(),
                    message: format!(
                        "review {} regra#{}: sensor '{}' fora do registro",
                        r.form, i, regra.sensor.nome
                    ),
                });
                continue;
            };
            if let Some(unit) = regra.threshold.unit {
                // unidade compatível com a grandeza do sensor (FORMAL §3)
                if !unit.grandeza().is_empty()
                    && !info.grandeza.is_empty()
                    && unit.grandeza() != info.grandeza
                {
                    diags.push(LoadDiag {
                        code: "unidade_incompativel".into(),
                        message: format!(
                            "review {} regra#{}: unidade '{}' incompatível com a grandeza '{}' do sensor '{}' (esperado por grandeza: '{}')",
                            r.form,
                            i,
                            unit.simbolo(),
                            info.grandeza,
                            regra.sensor.nome,
                            info.unidade
                        ),
                    });
                }
            } else {
                diags.push(LoadDiag {
                    code: "unidade_ausente".into(),
                    message: format!(
                        "review {} regra#{}: sensor '{}' tem grandeza '{}' — threshold exige unidade ({}), FORMAL §3/§6",
                        r.form, i, regra.sensor.nome, info.grandeza, info.unidade
                    ),
                });
            }
            for acao in &regra.actions {
                if let Action::Act { actor, .. } = acao {
                    if !registry.atores.contains_key(actor) {
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
    if let Some(main) = &programa.main {
        fn passe(stmts: &[Statement], registry: &crate::fxp::Registry, diags: &mut Vec<LoadDiag>) {
            for st in stmts {
                match st {
                    Statement::Act { actor, .. } => {
                        if !registry.atores.contains_key(actor) {
                            diags.push(LoadDiag {
                                code: "ator_nao_registrado".into(),
                                message: format!("main: ator '{actor}' fora do registro FXP"),
                            });
                        }
                    }
                    Statement::Every { body, .. } => passe(body, registry, diags),
                    Statement::Keep(_) => {}
                }
            }
        }
        passe(&main.statements, registry, &mut diags);
    }
    diags
}
