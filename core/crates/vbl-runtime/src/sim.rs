//! Simulador físico determinístico do FXP (esqueleto da Etapa 1, PLAN §6.5 —
//! evolui para módulo próprio na Etapa 3).
//!
//! - Séries temporais roteirizadas (`schedule`/`set_sensor`): determinismo
//!   total, sem aleatoriedade;
//! - Injeção de falhas: sensor ausente, registrado porém inacessível, ator
//!   que não responde (heartbeat);
//! - Registro mínimo obrigatório (FORMAL §6) com limites inclusivos;
//! - Política de fallback no REGISTRO (FORMAL §4.3) — o runtime não implementa
//!   fallback próprio;
//! - Mensagens FXP serializadas em processo (schema binário v1: Etapa 3).

use crate::fxp::{
    ActOutcome, ActorEffect, ActorLimits, ActorState, Fxp, Limit, Registry, SensorFailure,
    SensorInfo, SensorState, Value,
};
use crate::json::Json;
use crate::ledger::{kinds, Ledger};
use std::collections::BTreeMap;

/// Mensagem FXP serializada (fronteira em processo, sem schema binário).
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub seq: u64,
    pub op: String,
    pub actor: String,
    pub value: Value,
    pub tick: u64,
    /// Presente quando a entrega veio de um fallback.
    pub fallback_of: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SimulatorBuilder {
    registry: Registry,
    values: BTreeMap<String, f64>,
}

impl Default for SimulatorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatorBuilder {
    /// Registro mínimo (FORMAL §6) + valores iniciais plausíveis.
    pub fn new() -> Self {
        Self {
            registry: Registry::minimum(),
            values: BTreeMap::from([
                ("cpu_temp".to_string(), 55.0),
                ("cpu_power".to_string(), 150.0),
                ("attention".to_string(), 100.0),
            ]),
        }
    }

    pub fn with_sensor(mut self, name: &str, info: SensorInfo, value: f64) -> Self {
        self.registry.sensores.insert(name.into(), info);
        self.values.insert(name.into(), value);
        self
    }

    pub fn with_actor(mut self, name: &str, limits: ActorLimits) -> Self {
        self.registry.actors.insert(name.into(), limits);
        self
    }

    pub fn with_value(mut self, sensor: &str, value: f64) -> Self {
        self.values.insert(sensor.into(), value);
        self
    }

    pub fn build(self) -> FxpSimulator {
        let mut sim = FxpSimulator {
            registry: self.registry,
            sensores: BTreeMap::new(),
            actors: BTreeMap::new(),
            outbox: Vec::new(),
            delivered: Vec::new(),
            seq: 0,
            ticks: 0,
            schedule: BTreeMap::new(),
            disk_bytes_used: 1024,
        };
        for (name, value) in self.values {
            sim.sensores.insert(
                name.clone(),
                SensorState {
                    value,
                    accessible: true,
                },
            );
        }
        // Atores com efeitos físicos determinísticos (PLAN §6.5)
        for name in ["CpuPowerCap", "Fan", "StatusLed"] {
            if let Some(limits) = sim.registry.actors.get(name).cloned() {
                let effect = match name {
                    "CpuPowerCap" => ActorEffect::PowerCap,
                    "Fan" => ActorEffect::Fan,
                    _ => ActorEffect::NoEffect,
                };
                sim.actors.insert(
                    name.to_string(),
                    ActorState {
                        limits,
                        current: None,
                        available: true,
                        fallback: Vec::new(),
                        effect,
                    },
                );
            }
        }
        sim
    }
}

/// Barramento FXP simulado e determinístico.
#[derive(Debug, Clone)]
pub struct FxpSimulator {
    registry: Registry,
    sensores: BTreeMap<String, SensorState>,
    pub actors: BTreeMap<String, ActorState>,
    /// Mensagens serializadas (todo comando, mesmo rejeitado).
    pub outbox: Vec<Message>,
    /// Entregas efetivadas (ator correto).
    pub delivered: Vec<Message>,
    seq: u64,
    ticks: u64,
    /// Séries roteirizadas: tick (1-based) → valores absolutos de sensores.
    schedule: BTreeMap<u64, Vec<(String, f64)>>,
    disk_bytes_used: u64,
}

impl FxpSimulator {
    pub fn new() -> Self {
        SimulatorBuilder::new().build()
    }

    // ------------------------------------------------------------------
    // Roteirização do mundo e injeção de falhas (PLAN §6.5)
    // ------------------------------------------------------------------
    pub fn set_sensor(&mut self, name: &str, value: f64) {
        if let Some(s) = self.sensores.get_mut(name) {
            s.value = value;
            s.accessible = true;
        }
    }

    /// Sensor registrado porém inacessível (FORMAL §4.7).
    pub fn fail_sensor(&mut self, name: &str) {
        if let Some(s) = self.sensores.get_mut(name) {
            s.accessible = false;
        }
    }

    pub fn recover_sensor(&mut self, name: &str) {
        if let Some(s) = self.sensores.get_mut(name) {
            s.accessible = true;
        }
    }

    /// Remove sensor do registro (falha `sensor_nao_registrado`).
    pub fn unregister_sensor(&mut self, name: &str) {
        self.sensores.remove(name);
        self.registry.sensores.remove(name);
    }

    /// Agenda valor absoluto para um tick (1-based).
    pub fn schedule(&mut self, tick: u64, sensor: &str, value: f64) {
        self.schedule
            .entry(tick)
            .or_default()
            .push((sensor.into(), value));
    }

    /// Ator para de responder (heartbeat falho — BDD Caso 3).
    pub fn fail_actor(&mut self, name: &str) {
        if let Some(a) = self.actors.get_mut(name) {
            a.available = false;
        }
    }

    pub fn recover_actor(&mut self, name: &str) {
        if let Some(a) = self.actors.get_mut(name) {
            a.available = true;
        }
    }

    /// Política de fallback fica no REGISTRO do FXP (FORMAL §4.3).
    pub fn set_fallback(&mut self, primary: &str, alternativos: &[&str]) {
        if let Some(a) = self.actors.get_mut(primary) {
            a.fallback = alternativos.iter().map(|s| s.to_string()).collect();
        }
    }

    /// Registra ator extra (extensão opcional, ex.: `ReserveFan`).
    pub fn register_actor(&mut self, name: &str, limits: ActorLimits) {
        self.registry.actors.insert(name.into(), limits.clone());
        self.actors.insert(
            name.into(),
            ActorState {
                limits,
                current: None,
                available: true,
                fallback: Vec::new(),
                effect: ActorEffect::NoEffect,
            },
        );
    }

    /// Registra (ou substitui) um sensor no registro/simulador — usado pelo
    /// bus da Etapa 3 para sincronizar o `DeviceRegistry` (fonte única) com
    /// o backend simulado; valor inicial plausível é 0.0 até roteirização.
    pub fn register_sensor(&mut self, name: &str, info: SensorInfo) {
        self.registry.sensores.insert(name.into(), info);
        self.sensores.insert(
            name.into(),
            SensorState {
                value: 0.0,
                accessible: true,
            },
        );
    }

    // ------------------------------------------------------------------
    // Observação (testes/CLI)
    // ------------------------------------------------------------------
    pub fn current_actor(&self, name: &str) -> Option<&Value> {
        self.actors.get(name).and_then(|a| a.current.as_ref())
    }

    /// Registro atual (sensores/atores + limites) — FORMAL §6.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn sensor_value(&self, name: &str) -> Option<f64> {
        self.sensores.get(name).map(|s| s.value)
    }

    fn limit_violation(limits: &ActorLimits, value: f64) -> Option<(Limit, f64)> {
        // limites INCLUSIVOS: valor igual ao limite é aceito (FORMAL §4.3)
        if let Some(min) = limits.min {
            if value < min {
                return Some((Limit::Min, min));
            }
        }
        if let Some(max) = limits.max {
            if value > max {
                return Some((Limit::Max, max));
            }
        }
        if let Some(safety) = limits.safety_limit {
            if value > safety {
                return Some((Limit::SafetyLimit, safety));
            }
        }
        None
    }

    fn deliver(&mut self, actor: &str, value: &Value, msg: Message, ledger: &mut dyn Ledger) {
        let effect = self.actors.get(actor).map(|a| a.effect).unwrap_or_default();
        match effect {
            ActorEffect::PowerCap => {
                if let Some(v) = value.as_num() {
                    let current = self.sensor_value("cpu_power").unwrap_or(v);
                    if v < current {
                        self.set_sensor("cpu_power", v);
                    }
                }
            }
            ActorEffect::Fan => {
                if let Some(v) = value.as_num() {
                    if let Some(t) = self.sensor_value("cpu_temp") {
                        let new = (t - (v / 255.0) * 8.0).max(0.0);
                        self.set_sensor("cpu_temp", new);
                    }
                }
            }
            ActorEffect::NoEffect => {}
        }
        if let Some(a) = self.actors.get_mut(actor) {
            a.current = Some(value.clone());
        }
        self.delivered.push(msg);
        ledger.actuator_action(actor, value, true);
    }
}

impl Default for FxpSimulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Fxp for FxpSimulator {
    fn read_sensor(&mut self, name: &str, ledger: &mut dyn Ledger) -> Result<f64, SensorFailure> {
        let sensor = self.sensores.get(name);
        match sensor {
            None => {
                ledger.alert(
                    &format!(
                        "Sensor '{name}' não registrado no FXP (falha de I/O). Condição não avaliada neste tick."
                    ),
                    Json::obj([
                        ("motivo", Json::str("sensor_not_registered")),
                        ("sensor", Json::str(name)),
                    ]),
                );
                Err(SensorFailure::NotRegistered)
            }
            Some(s) if !s.accessible => {
                ledger.alert(
                    &format!(
                        "Sensor '{name}' registrado porém inacessível (falha de leitura). Condição não avaliada neste tick."
                    ),
                    Json::obj([
                        ("motivo", Json::str("sensor_inaccessible")),
                        ("sensor", Json::str(name)),
                    ]),
                );
                Err(SensorFailure::Inaccessible)
            }
            Some(s) => Ok(arredondar2(s.value)),
        }
    }

    fn act(&mut self, actor: &str, value: Value, ledger: &mut dyn Ledger) -> ActOutcome {
        self.seq += 1;
        let msg = Message {
            seq: self.seq,
            op: "act".into(),
            actor: actor.into(),
            value: value.clone(),
            tick: self.ticks,
            fallback_of: None,
        };
        self.outbox.push(msg);

        let Some(state) = self.actors.get(actor).cloned() else {
            ledger.record(
                kinds::ACTOR_UNKNOWN,
                &format!("Ator '{actor}' não registrado no FXP."),
                Json::obj([("ator", Json::str(actor))]),
            );
            ledger.actuator_action(actor, &value, false);
            return ActOutcome::MissingActor;
        };

        if !state.available {
            ledger.actuator_action(actor, &value, false);
            ledger.record(
                kinds::ACTOR_UNAVAILABLE,
                &format!("Heartbeat do ator '{actor}' não respondeu."),
                Json::obj([("ator", Json::str(actor))]),
            );
            return self.try_fallback(actor, &value, &state.fallback, ledger);
        }

        if let Some(v) = value.as_num() {
            if let Some((limit, limit_value)) = Self::limit_violation(&state.limits, v) {
                ledger.record(
                    kinds::ACTOR_REJECTED_VALUE,
                    &format!(
                        "Comando a '{actor}' rejeitado sem envio: valor {v} viola {} = {limit_value}.",
                        limit.name()
                    ),
                    Json::obj([
                        ("ator", Json::str(actor)),
                        ("valor", Json::num(v)),
                        ("limite", Json::str(limit.name())),
                        ("limite_valor", Json::num(limit_value)),
                    ]),
                );
                ledger.actuator_action(actor, &value, false);
                return ActOutcome::Rejected { limit, limit_value };
            }
        }

        let msg = self.outbox.last().unwrap().clone();
        self.deliver(actor, &value, msg, ledger);
        ActOutcome::Delivered
    }

    fn cpu_power(&self) -> f64 {
        self.sensores
            .get("cpu_power")
            .map(|s| s.value)
            .unwrap_or(0.0)
    }

    fn on_tick(&mut self, _ledger: &mut dyn Ledger) {
        self.ticks += 1;
        if let Some(values) = self.schedule.remove(&self.ticks) {
            for (name, value) in values {
                self.set_sensor(&name, value);
            }
        }
    }

    fn disk_bytes_used(&self) -> u64 {
        self.disk_bytes_used
    }

    fn add_disk_bytes(&mut self, n: u64) {
        self.disk_bytes_used += n;
    }

    fn registry(&self) -> &Registry {
        &self.registry
    }
}

impl FxpSimulator {
    fn try_fallback(
        &mut self,
        primary: &str,
        value: &Value,
        fallback: &[String],
        ledger: &mut dyn Ledger,
    ) -> ActOutcome {
        for alt in fallback {
            let Some(alt_limits) = self.registry.actors.get(alt).cloned() else {
                continue;
            };
            let available = self.actors.get(alt).map(|a| a.available).unwrap_or(false);
            if !available {
                continue;
            }
            if let Some(v) = value.as_num() {
                if Self::limit_violation(&alt_limits, v).is_some() {
                    ledger.alert(
                        &format!("Fallback '{alt}' rejeitou o valor {v} (limites)."),
                        Json::obj([
                            ("motivo", Json::str("fallback_rejeitado")),
                            ("ator", Json::str(alt)),
                        ]),
                    );
                    continue;
                }
            }
            let msg = Message {
                seq: self.seq,
                op: "act".into(),
                actor: alt.clone(),
                value: value.clone(),
                tick: self.ticks,
                fallback_of: Some(primary.into()),
            };
            self.outbox.push(msg.clone());
            self.deliver(alt, value, msg, ledger);
            ledger.record(
                kinds::FALLBACK_EXECUTED,
                &format!("Fallback '{alt}' acionado após falha de '{primary}'."),
                Json::obj([
                    ("primario", Json::str(primary)),
                    ("alternativo", Json::str(alt)),
                    ("valor", value.to_json()),
                ]),
            );
            return ActOutcome::FallbackExecuted {
                alternativo: alt.clone(),
            };
        }
        ledger.alert(
            &format!("Todos os fallbacks de '{primary}' falharam."),
            Json::obj([
                ("motivo", Json::str("fallback_esgotado")),
                ("ator", Json::str(primary)),
            ]),
        );
        ActOutcome::FallbackExhausted
    }
}

/// Arredonda para 2 casas (mesma fidelidade do protótipo da Etapa 1).
fn arredondar2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
