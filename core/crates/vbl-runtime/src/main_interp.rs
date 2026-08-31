//! Bloco `main` — intérprete de `keep`/`act`/`every` (FORMAL §3).
//!
//! Statements de topo do `main` rodam como um bloco `every` de período = 1
//! tick (contrato do loader da Etapa 1); blocos `every` vencidos executam
//! uma vez por tick (`run_due`, chamado antes do tick do engine).

use crate::notebook::{kinds, Caderno};
use crate::engine::Engine;
use crate::fxp::{Fxp, Value};
use vbl_lang::Conjugation;

/// Statement compilado do bloco `main`.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtRt {
    Keep(String),
    Act { ator: String, valor: Value },
    Every { periodo_s: f64, body: Vec<StmtRt> },
}

/// Bloco `every` com estado de agendamento (`next_due`).
#[derive(Debug, Clone)]
pub struct EveryBlock {
    pub periodo_s: f64,
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
    pub fn add_every(&mut self, periodo_s: f64, statements: Vec<StmtRt>) {
        self.every_blocks
            .push(EveryBlock { periodo_s, next_due: periodo_s, statements });
    }

    /// Executa os blocos `every` vencidos; chamado uma vez por tick,
    /// ANTES do `engine.tick()` (coreografia da demo da Etapa 1).
    pub fn run_due<F: Fxp, C: Caderno>(&mut self, engine: &mut Engine<F, C>) {
        let now = engine.sim_time;
        let vencidos: Vec<usize> = (0..self.every_blocks.len())
            .filter(|&i| now + 1e-9 >= self.every_blocks[i].next_due)
            .collect();
        for i in vencidos {
            let statements = self.every_blocks[i].statements.clone();
            for st in &statements {
                self.run_statement(engine, st);
            }
            self.every_blocks[i].next_due += self.every_blocks[i].periodo_s;
        }
    }

    fn run_statement<F: Fxp, C: Caderno>(&mut self, engine: &mut Engine<F, C>, st: &StmtRt) {
        match st {
            StmtRt::Keep(forma) => {
                let Some(form) = engine.forma(forma) else {
                    // cláusula de erro: keep de forma inexistente/dissolvida —
                    // registrado no Caderno, sem interromper o runtime
                    engine.caderno.record(
                        kinds::KEEP_FORMA_INEXISTENTE,
                        &format!("keep('{forma}'): forma inexistente ou já dissolvida."),
                        Json::obj([("forma", Json::str(forma))]),
                    );
                    return;
                };
                if form.conjugation == Conjugation::Nonequilibrium {
                    let agora = engine.sim_time;
                    let versao = {
                        let form = engine.forma_mut(forma).unwrap();
                        form.keep(agora);
                        form.manutencao_versao += 1;
                        form.manutencao_versao
                    };
                    let prazo = engine
                        .forma(forma)
                        .and_then(|f| f.manutencao.as_ref().map(|m| m.ultima + m.deadline_s))
                        .unwrap_or(engine.sim_time);
                    engine.scheduler.agendar(forma, crate::scheduler::Prazo::Manutencao, prazo, versao);
                } else {
                    let conj = engine.forma(forma).map(|f| f.conjugation).unwrap_or(Conjugation::Event);
                    engine.caderno.record(
                        kinds::KEEP_IGNORADO,
                        &format!("keep('{forma}'): conjugação {} não exige manutenção.", conj.nome()),
                        Json::obj([("forma", Json::str(forma))]),
                    );
                }
            }
            StmtRt::Act { ator, valor } => {
                engine.fxp.act(ator, valor.clone(), &mut engine.caderno);
            }
            StmtRt::Every { .. } => unreachable!("blocos every vivem em every_blocks"),
        }
    }
}

use crate::json::Json;
