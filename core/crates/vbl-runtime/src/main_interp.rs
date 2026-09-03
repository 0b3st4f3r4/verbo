//! Bloco `main` — intérprete de `keep`/`act`/`every` (FORMAL §3).
//!
//! Statements de topo do `main` rodam como um bloco `every` de período = 1
//! tick (contrato do loader da Etapa 1); blocos `every` vencidos executam
//! uma vez por tick (`run_due`, chamado antes do tick do engine).

use crate::engine::Engine;
use crate::fxp::{Fxp, Value};
use crate::ledger::{kinds, Ledger};
use vbl_lang::Conjugation;

/// Statement compilado do bloco `main`.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtRt {
    Keep(String),
    Act { actor: String, value: Value },
    Every { period_s: f64, body: Vec<StmtRt> },
}

/// Bloco `every` com estado de agendamento (`next_due`).
#[derive(Debug, Clone)]
pub struct EveryBlock {
    pub period_s: f64,
    pub next_due: f64,
    pub statements: Vec<StmtRt>,
}

/// Intérprete do bloco `main`.
#[derive(Debug, Clone, Default)]
pub struct MainInterpreter {
    pub every_blocks: Vec<EveryBlock>,
}

impl MainInterpreter {
    /// Registra um bloco `every <periodo> { <statements> }`.
    pub fn add_every(&mut self, period_s: f64, statements: Vec<StmtRt>) {
        self.every_blocks.push(EveryBlock {
            period_s,
            next_due: period_s,
            statements,
        });
    }

    /// Executa os blocos `every` vencidos; chamado uma vez por tick,
    /// ANTES do `engine.tick()` (coreografia da demo da Etapa 1).
    pub fn run_due<F: Fxp, C: Ledger>(&mut self, engine: &mut Engine<F, C>) {
        let now = engine.sim_time;
        let due: Vec<usize> = (0..self.every_blocks.len())
            .filter(|&i| now + 1e-9 >= self.every_blocks[i].next_due)
            .collect();
        for i in due {
            let statements = self.every_blocks[i].statements.clone();
            for st in &statements {
                self.run_statement(engine, st);
            }
            self.every_blocks[i].next_due += self.every_blocks[i].period_s;
        }
    }

    fn run_statement<F: Fxp, C: Ledger>(&mut self, engine: &mut Engine<F, C>, st: &StmtRt) {
        match st {
            StmtRt::Keep(name) => {
                let Some(form) = engine.form(name) else {
                    // cláusula de erro: keep de forma inexistente/dissolvida —
                    // registrado no Caderno, sem interromper o runtime
                    engine.ledger.record(
                        kinds::KEEP_UNKNOWN_FORM,
                        &format!("keep('{name}'): forma inexistente ou já dissolvida."),
                        Json::obj([("forma", Json::str(name))]),
                    );
                    return;
                };
                if form.conjugation == Conjugation::Nonequilibrium {
                    let now = engine.sim_time;
                    let version = {
                        let form = engine.form_mut(name).unwrap();
                        form.keep(now);
                        form.maintenance_version += 1;
                        form.maintenance_version
                    };
                    let deadline = engine
                        .form(name)
                        .and_then(|f| f.maintenance.as_ref().map(|m| m.last + m.deadline_s))
                        .unwrap_or(engine.sim_time);
                    engine.scheduler.schedule(
                        name,
                        crate::scheduler::Deadline::Maintenance,
                        deadline,
                        version,
                    );
                } else {
                    let conj = engine
                        .form(name)
                        .map(|f| f.conjugation)
                        .unwrap_or(Conjugation::Event);
                    engine.ledger.record(
                        kinds::KEEP_IGNORED,
                        &format!(
                            "keep('{name}'): conjugação {} não exige manutenção.",
                            conj.name()
                        ),
                        Json::obj([("forma", Json::str(name))]),
                    );
                }
            }
            StmtRt::Act { actor, value } => {
                engine.fxp.act(actor, value.clone(), &mut engine.ledger);
            }
            StmtRt::Every { .. } => unreachable!("blocos every vivem em every_blocks"),
        }
    }
}

use crate::json::Json;
