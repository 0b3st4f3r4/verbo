//! Roteirização do mundo simulado (sensores/atores) para o `vbl run` —
//! determinística, equivalente ao `FXPSimulator` da suíte (PLAN §6.5).
//! Etapa 4: injeção de falha de ator, fallback do registro e atores extras
//! para os cenários E2E (BDD Caso 3) direto do CLI.

use vbl_runtime::FxpSimulator;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Script {
    initial: BTreeMap<String, f64>,
    /// tick (1-based) → [(sensor, valor absoluto)]
    schedule: BTreeMap<u64, Vec<(String, f64)>>,
    /// Atores que param de responder (heartbeat falho — BDD Caso 3).
    failed_actors: Vec<String>,
    /// Política de fallback do registro (FORMAL §4.3): primário → alternativo.
    fallbacks: BTreeMap<String, String>,
    /// Atores extras registrados (extensões opcionais, ex.: ReserveFan).
    extra_actors: Vec<String>,
}

impl Script {
    pub fn set(&mut self, sensor: &str, value: f64) {
        self.initial.insert(sensor.into(), value);
    }

    pub fn at(&mut self, tick: u64, sensor: &str, value: f64) {
        self.schedule.entry(tick).or_default().push((sensor.into(), value));
    }

    pub fn fail_actor(&mut self, actor: &str) {
        self.failed_actors.push(actor.into());
    }

    pub fn fallback(&mut self, primary: &str, alternativo: &str) {
        self.fallbacks.insert(primary.into(), alternativo.into());
    }

    pub fn register_actor(&mut self, actor: &str) {
        self.extra_actors.push(actor.into());
    }

    pub fn build_simulator(&self) -> FxpSimulator {
        let mut fxp = FxpSimulator::new();
        for (sensor, value) in &self.initial {
            fxp.set_sensor(sensor, *value);
        }
        for (tick, values) in &self.schedule {
            for (sensor, value) in values {
                fxp.schedule(*tick, sensor, *value);
            }
        }
        for actor in &self.extra_actors {
            fxp.register_actor(
                actor,
                vbl_runtime::fxp::ActorLimits {
                    min: Some(0.0),
                    max: Some(255.0),
                    safety_limit: Some(200.0),
                },
            );
        }
        for (primary, alternativo) in &self.fallbacks {
            fxp.set_fallback(primary, &[alternativo]);
        }
        for actor in &self.failed_actors {
            fxp.fail_actor(actor);
        }
        fxp
    }

    /// O roteiro acabou? (nenhum tick futuro agendado)
    pub fn finished(&self, clock: u64) -> bool {
        self.schedule.keys().all(|t| *t <= clock)
    }
}
