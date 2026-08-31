//! Roteirização do mundo simulado (sensores/atores) para o `vbl run` —
//! determinística, equivalente ao `FXPSimulator` da suíte (PLAN §6.5).
//! Etapa 4: injeção de falha de ator, fallback do registro e atores extras
//! para os cenários E2E (BDD Caso 3) direto do CLI.

use vbl_runtime::FxpSimulator;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Roteiro {
    iniciais: BTreeMap<String, f64>,
    /// tick (1-based) → [(sensor, valor absoluto)]
    cronograma: BTreeMap<u64, Vec<(String, f64)>>,
    /// Atores que param de responder (heartbeat falho — BDD Caso 3).
    atores_falhos: Vec<String>,
    /// Política de fallback do registro (FORMAL §4.3): primário → alternativo.
    fallbacks: BTreeMap<String, String>,
    /// Atores extras registrados (extensões opcionais, ex.: VentoinhaReserva).
    atores_extras: Vec<String>,
}

impl Roteiro {
    pub fn set(&mut self, sensor: &str, valor: f64) {
        self.iniciais.insert(sensor.into(), valor);
    }

    pub fn at(&mut self, tick: u64, sensor: &str, valor: f64) {
        self.cronograma.entry(tick).or_default().push((sensor.into(), valor));
    }

    pub fn falhar_ator(&mut self, ator: &str) {
        self.atores_falhos.push(ator.into());
    }

    pub fn fallback(&mut self, primario: &str, alternativo: &str) {
        self.fallbacks.insert(primario.into(), alternativo.into());
    }

    pub fn registrar_ator(&mut self, ator: &str) {
        self.atores_extras.push(ator.into());
    }

    pub fn construir_simulador(&self) -> FxpSimulator {
        let mut fxp = FxpSimulator::novo();
        for (sensor, valor) in &self.iniciais {
            fxp.set_sensor(sensor, *valor);
        }
        for (tick, valores) in &self.cronograma {
            for (sensor, valor) in valores {
                fxp.programar(*tick, sensor, *valor);
            }
        }
        for ator in &self.atores_extras {
            fxp.registrar_ator(
                ator,
                vbl_runtime::fxp::ActorLimits {
                    min: Some(0.0),
                    max: Some(255.0),
                    safety_limit: Some(200.0),
                },
            );
        }
        for (primario, alternativo) in &self.fallbacks {
            fxp.definir_fallback(primario, &[alternativo]);
        }
        for ator in &self.atores_falhos {
            fxp.falhar_ator(ator);
        }
        fxp
    }

    /// O roteiro acabou? (nenhum tick futuro agendado)
    pub fn terminou(&self, clock: u64) -> bool {
        self.cronograma.keys().all(|t| *t <= clock)
    }
}
