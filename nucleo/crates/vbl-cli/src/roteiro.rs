//! Roteirização do mundo simulado (sensores) para o `vbl run` — determinística,
//! equivalente ao `FXPSimulator` da suíte (PLAN §6.5).

use vbl_runtime::FxpSimulator;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Roteiro {
    iniciais: BTreeMap<String, f64>,
    /// tick (1-based) → [(sensor, valor absoluto)]
    cronograma: BTreeMap<u64, Vec<(String, f64)>>,
}

impl Roteiro {
    pub fn set(&mut self, sensor: &str, valor: f64) {
        self.iniciais.insert(sensor.into(), valor);
    }

    pub fn at(&mut self, tick: u64, sensor: &str, valor: f64) {
        self.cronograma.entry(tick).or_default().push((sensor.into(), valor));
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
        fxp
    }

    /// O roteiro acabou? (nenhum tick futuro agendado)
    pub fn terminou(&self, clock: u64) -> bool {
        self.cronograma.keys().all(|t| *t <= clock)
    }

    pub fn aplicar_antes_do_tick(&self, _tick: u64, _fxp: &mut FxpSimulator) {
        // (a roteirização absoluta é aplicada dentro do `on_tick` do simulador)
    }
}
