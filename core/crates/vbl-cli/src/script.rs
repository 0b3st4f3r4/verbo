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

// ── suíte do roteiro do mundo simulado ────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use vbl_runtime::fxp::{ActOutcome, Fxp as _, Value};
    use vbl_runtime::ledger::ChainLedger;

    #[test]
    fn roteiro_completo_constrói_simulador_determinístico() {
        let mut script = Script::default();
        script.set("cpu_temp", 90.0);
        script.at(3, "attention", 15.0);
        script.at(3, "cpu_power", 420.0);
        script.fail_actor("Fan");
        script.fallback("Fan", "ReserveFan");
        script.register_actor("ReserveFan");
        let mut ledger = ChainLedger::new();
        let mut sim = script.build_simulator();
        // valor inicial aplicado no build
        assert_eq!(sim.read_sensor("cpu_temp", &mut ledger), Ok(90.0));
        // agendado para o tick 3 ainda não vale antes…
        assert_eq!(sim.read_sensor("attention", &mut ledger), Ok(100.0));
        sim.on_tick(&mut ledger);
        sim.on_tick(&mut ledger);
        assert_eq!(sim.read_sensor("attention", &mut ledger), Ok(100.0));
        sim.on_tick(&mut ledger);
        // …e vale a partir do tick roteirizado
        assert_eq!(sim.read_sensor("attention", &mut ledger), Ok(15.0));
        assert_eq!(sim.read_sensor("cpu_power", &mut ledger), Ok(420.0));
        // ator extra registrado responde na faixa declarada (0..255, safety 200)
        assert!(matches!(
            sim.act("ReserveFan", Value::Num(150.0), &mut ledger),
            ActOutcome::Delivered
        ));
        // o roteiro terminou no tick 3 (nada agendado depois)
        assert!(script.finished(3));
        assert!(!script.finished(2));
    }
}
