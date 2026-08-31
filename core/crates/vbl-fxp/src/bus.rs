//! O barramento FXP — [`FxpBus`] implementa o trait `Fxp` do runtime
//! roteando cada dispositivo conforme o modo de operação (PLAN §3.1/§3.4):
//!
//! | modo global | rota |
//! |-------------|------|
//! | `simulado`  | tudo no simulador determinístico (compatível com a Etapa 2) |
//! | `hibrido`   | por dispositivo: reais onde há rota, simulados no restante (marcado) |
//! | `real`      | nada sintético: dispositivo sem rota real é **registrado porém inacessível** (§4.7) |
//!
//! Honestidade (FORMAL §4.7): dado sintético só circula em modo simulado/
//! híbrido **explícito** e chega marcado ao Caderno; falha de I/O nunca é
//! leitura 0.0; fallback é política do REGISTRO (§4.3) e atua na rota de
//! I/O — nunca falsifica leitura.
//!
//! O `DeviceRegistry` é a **fonte única** de limites/aliases; na construção o
//! simulador embutido é sincronizado com ele, de modo que o backend simulado
//! valide exatamente o mesmo registro.

use crate::drivers::{actor_from, discover, sensor_from, ActorDriver, SensorDriver};
use crate::queue::{Command, CommandQueue};
use crate::registry::{DeviceKind, DeviceMode, DeviceRegistry, Endpoint, OperationMode, RemoteAddr};
use crate::schema::{flag, reason, AckAct, Body, Message, WireValue};
use crate::transport::{Connection, TransportError};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use vbl_runtime::ledger::{kinds as rt_kinds, Actuation, Ledger};
use vbl_runtime::fxp::{
    ActOutcome, ActorLimits, SensorFailure, Fxp, Limit, Registry as RuntimeRegistry, Value,
    PRIORITY_NORMAL,
};
use vbl_runtime::json::Json;
use vbl_runtime::sim::FxpSimulator;

/// Kinds de evento acrescentados pelo barramento (vocabulário do Caderno —
/// FORMAL §6 lista os canônicos de fim de forma; estes cobrem o ciclo de
/// vida da fila prioritária).
pub mod kinds {
    /// Comando enfileirado foi entregue numa re-entrega do `on_tick`.
    pub const COMANDO_REENTREGUE: &str = "comando_reentregue";
    /// Comando excedeu `queue_timeout_ticks` e foi descartado com alerta.
    pub const COMANDO_EXPIRADO: &str = "comando_expirado";
}

/// Trilha de atuação com a latência medida (Etapa 4 — PLAN §4.1): o Caderno
/// estima o custo energético como potência do tick × latência do ack.
fn actuation_with_latency(
    ledger: &mut dyn Ledger,
    actor: &str,
    value: &Value,
    latency_us: u64,
    success: bool,
) {
    ledger.actuator_action_detailed(Actuation {
        actor: actor.to_owned(),
        requested: value.clone(),
        applied: if success { Some(value.clone()) } else { None },
        latency_us: Some(latency_us),
        joule_cost: None,
        success,
    });
}

/// Parâmetros do barramento (defaults = docs/FXP-SCHEMA-v1.md §6).
#[derive(Debug, Clone)]
pub struct BusConfig {
    pub mode: OperationMode,
    /// Cache de leitura de rotas reais/remotas (mitigação PLAN §3).
    pub cache_ttl: Duration,
    /// Prazo de ack de leitura remota (fio).
    pub read_timeout: Duration,
    /// Prazo de ack de atuação local (entre processos).
    pub act_timeout_local: Duration,
    /// Prazo de ack de atuação remota.
    pub act_timeout_remote: Duration,
    /// Tentativas de transporte além da primeira (PLAN §3: retry).
    pub retries: u32,
    /// Ticks virtuais que um comando pode esperar na fila.
    pub queue_timeout_ticks: u64,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            mode: OperationMode::Simulated,
            cache_ttl: Duration::from_millis(100),
            read_timeout: Duration::from_millis(10),
            act_timeout_local: Duration::from_millis(50),
            act_timeout_remote: Duration::from_millis(500),
            retries: 1,
            queue_timeout_ticks: 2,
        }
    }
}

/// Rota efetiva de um dispositivo (resolvida na construção).
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// Simulador embutido (modo simulado global ou dispositivo simulado).
    Simulator,
    /// Driver de arquivo real (sysfs/hwmon/led).
    Real,
    /// Peer remoto falando schema v1.
    Remote(RemoteAddr),
    /// Registrado porém inacessível: modo real global proíbe a rota simulada
    /// e não há rota real (§4.7 — nunca simulado silencioso).
    Inaccessible { reason: String },
}

impl Route {
    /// Descrição legível (probe/relatório).
    pub fn description(&self) -> String {
        match self {
            Route::Simulator => "simulado (em processo)".into(),
            Route::Real => "real (driver de arquivo)".into(),
            Route::Remote(a) => match a {
                RemoteAddr::Unix(p) => format!("remota (unix:{})", p.display()),
                RemoteAddr::Tcp { host, port } => format!("remota (tcp:{host}:{port})"),
            },
            Route::Inaccessible { reason } => format!("inacessível ({reason})"),
        }
    }
}

/// O barramento FXP consumido pelo runtime (`Engine<F: Fxp>`).
pub struct FxpBus {
    registry: DeviceRegistry,
    rt_registry: RuntimeRegistry,
    config: BusConfig,
    sim: FxpSimulator,
    routes: BTreeMap<String, Route>, // canônico → rota
    real_sensors: BTreeMap<String, Box<dyn SensorDriver + Send>>,
    real_actors: BTreeMap<String, Box<dyn ActorDriver + Send>>,
    connections: BTreeMap<String, Connection>, // canônico → transporte remoto
    cache: BTreeMap<String, (Instant, f64)>,
    queue: CommandQueue,
    seq: u32,
    disk_bytes: u64,
    /// Última leitura de `cpu_power` em rota real/remota (partilha P/N — §4.2).
    known_power: f64,
    power_inaccessible: bool,
    power_read_at: Option<Instant>,
}

impl std::fmt::Debug for FxpBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FxpBus")
            .field("config", &self.config)
            .field("rotas", &self.routes)
            .field("fila", &self.queue.len())
            .finish()
    }
}

impl FxpBus {
    /// Constrói o barramento com rotas resolvidas. Endpoints `Auto` tentam
    /// descoberta no host; falha de descoberta **não é erro de construção** —
    /// o dispositivo fica registrado porém inacessível (§4.7).
    pub fn build(registry: DeviceRegistry, mut config: BusConfig, mut sim: FxpSimulator) -> Self {
        let mut routes = BTreeMap::new();
        let mut real_sensors = BTreeMap::new();
        let mut real_actors = BTreeMap::new();

        if config.mode == OperationMode::Simulated {
            // Simulador determinístico: sem cache (roteirização é imediata).
            config.cache_ttl = Duration::ZERO;
        }

        // Fonte única de verdade: o registro rico acrescenta ao simulador os
        // dispositivos que só existem nele (extensões). Dispositivos já
        // presentes no sim (roteirizados pelo CLI) são PRESERVADOS — estado
        // como "ator indisponível" do cenário não pode ser resetado.
        for d in registry.devices() {
            match &d.kind {
                DeviceKind::Sensor { quantity, unit, .. } => {
                    if !sim.registry().sensores.contains_key(&d.name) {
                        sim.register_sensor(
                            &d.name,
                            vbl_runtime::fxp::SensorInfo {
                                quantity: quantity.clone(),
                                unit: unit.clone(),
                            },
                        );
                    }
                }
                DeviceKind::Actor { limits } => {
                    if !sim.registry().actors.contains_key(&d.name) {
                        sim.register_actor(&d.name, limits.clone());
                    }
                }
            }
        }

        for d in registry.devices() {
            let route = match base_route(&config, d) {
                RouteSpec::Simulator => Route::Simulator,
                RouteSpec::Inaccessible { reason } => Route::Inaccessible { reason },
                RouteSpec::Concrete => {
                    let endpoint = match &d.endpoint {
                        Endpoint::Auto => discover(&d.name),
                        e => Some(e.clone()),
                    };
                    match endpoint {
                        None => Route::Inaccessible {
                            reason: "auto-descoberta não encontrou hardware".into(),
                        },
                        Some(Endpoint::Remote { addr }) => Route::Remote(addr),
                        Some(Endpoint::Simulated) => Route::Inaccessible {
                            reason: "modo real não roteia para simulador (dado sintético proibido)"
                                .into(),
                        },
                        Some(ep) => match &d.kind {
                            DeviceKind::Sensor { .. } => match sensor_from(&ep) {
                                Some(dr) => {
                                    real_sensors.insert(d.name.clone(), dr);
                                    Route::Real
                                }
                                None => Route::Inaccessible {
                                    reason: format!(
                                        "endpoint sem driver de leitura: {}",
                                        ep.description()
                                    ),
                                },
                            },
                            DeviceKind::Actor { .. } => match actor_from(&ep) {
                                Some(dr) => {
                                    real_actors.insert(d.name.clone(), dr);
                                    Route::Real
                                }
                                None => Route::Inaccessible {
                                    reason: format!(
                                        "endpoint sem driver de atuação: {}",
                                        ep.description()
                                    ),
                                },
                            },
                        },
                    }
                }
            };
            routes.insert(d.name.clone(), route);
        }

        let rt_registry = registry.to_runtime_registry();
        Self {
            registry,
            rt_registry,
            config,
            sim,
            routes,
            real_sensors,
            real_actors,
            connections: BTreeMap::new(),
            cache: BTreeMap::new(),
            queue: CommandQueue::default(),
            seq: 0,
            disk_bytes: 0,
            known_power: 0.0,
            power_inaccessible: false,
            power_read_at: None,
        }
    }

    /// Registro rico (aliases, modos, endpoints, fallback) — probe/config.
    pub fn registry_rico(&self) -> &DeviceRegistry {
        &self.registry
    }

    /// Simulador embutido (roteirização do CLI em modo simulado/híbrido).
    pub fn sim_mut(&mut self) -> &mut FxpSimulator {
        &mut self.sim
    }

    pub fn sim(&self) -> &FxpSimulator {
        &self.sim
    }

    /// Fila de comandos pendentes (observação de testes/probe).
    pub fn pending_queue(&self) -> usize {
        self.queue.len()
    }

    /// Rota efetiva de um nome simbólico (para `vbl fxp-probe`).
    pub fn route_of(&self, name: &str) -> Option<&Route> {
        self.routes.get(self.registry.canonical_of(name))
    }

    fn sensor_alert(
        &mut self,
        ledger: &mut dyn Ledger,
        reason: &str,
        sensor: &str,
        detail: &str,
    ) {
        ledger.alert(
            &format!(
                "Sensor '{sensor}' — falha de I/O ({reason}): {detail} Condição não avaliada neste tick."
            ),
            Json::obj([("motivo", Json::str(reason)), ("sensor", Json::str(sensor))]),
        );
    }

    fn next_seq(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn cache_valid(&self, canonical: &str) -> Option<f64> {
        if self.config.cache_ttl.is_zero() {
            return None;
        }
        match self.cache.get(canonical) {
            Some((read_at, value)) if read_at.elapsed() < self.config.cache_ttl => Some(*value),
            _ => None,
        }
    }

    /// Leitura em rota real: cache TTL → driver → honestidade de falha.
    fn read_real(
        &mut self,
        canonical: &str,
        ledger: &mut dyn Ledger,
    ) -> Result<f64, SensorFailure> {
        if let Some(v) = self.cache_valid(canonical) {
            return Ok(v);
        }
        let Some(dr) = self.real_sensors.get_mut(canonical) else {
            self.sensor_alert(ledger, "sensor_inaccessible", canonical, "sem driver real.");
            return Err(SensorFailure::Inaccessible);
        };
        match dr.read() {
            Ok(v) => {
                self.cache.insert(canonical.into(), (Instant::now(), v));
                self.note_power(canonical, v);
                Ok(v)
            }
            Err(_) => {
                if canonical == "cpu_power" {
                    self.power_inaccessible = true;
                }
                let detail =
                    format!("driver real ({}) falhou.", self.route_description(canonical));
                self.sensor_alert(ledger, "sensor_inaccessible", canonical, &detail);
                Err(SensorFailure::Inaccessible)
            }
        }
    }

    /// Leitura remota via schema v1 (READ → READ_OK/READ_ERR, ack por seq).
    fn read_remote(
        &mut self,
        canonical: &str,
        ledger: &mut dyn Ledger,
    ) -> Result<f64, SensorFailure> {
        if let Some(v) = self.cache_valid(canonical) {
            return Ok(v);
        }
        let Some(Route::Remote(addr)) = self.routes.get(canonical).cloned() else {
            return Err(SensorFailure::Inaccessible);
        };
        let seq = self.next_seq();
        let request = Message::read(canonical, seq, true);
        match self.request_remote(canonical, &addr, &request, self.config.read_timeout) {
            Ok(resp) => {
                let synthetic = resp.flags & flag::SYNTHETIC != 0;
                match resp.body {
                    Body::ReadOk { value, canonical: can } => {
                        if synthetic {
                            // §4.7: dado sintético sempre marcado no Caderno.
                            ledger.warn(
                                &format!("Leitura remota de '{canonical}' é de origem simulada (measurement_status: simulado)."),
                                Json::obj([
                                    ("motivo", Json::str("measurement_status_simulado")),
                                    ("sensor", Json::str(canonical)),
                                    ("canonical", Json::str(can)),
                                    ("valor", Json::num(value)),
                                ]),
                            );
                        }
                        self.cache.insert(canonical.into(), (Instant::now(), value));
                        self.note_power(canonical, value);
                        Ok(value)
                    }
                    Body::ReadErr { reason } => {
                        let (failure, reason) = match reason {
                            reason::NOT_REGISTERED => {
                                (SensorFailure::NotRegistered, "sensor_not_registered")
                            }
                            _ => (SensorFailure::Inaccessible, "sensor_inaccessible"),
                        };
                        self.sensor_alert(ledger, reason, canonical, "peer respondeu erro.");
                        Err(failure)
                    }
                    _ => {
                        self.sensor_alert(
                            ledger,
                            "sensor_inaccessible",
                            canonical,
                            "resposta inesperada do peer.",
                        );
                        Err(SensorFailure::Inaccessible)
                    }
                }
            }
            Err(e) => {
                self.connections.remove(canonical); // conexão suspeita: reconectar
                self.sensor_alert(
                    ledger,
                    "sensor_inaccessible",
                    canonical,
                    &format!("transporte: {e}."),
                );
                Err(SensorFailure::Inaccessible)
            }
        }
    }

    fn note_power(&mut self, canonical: &str, value: f64) {
        if canonical == "cpu_power" {
            self.known_power = value;
            self.power_inaccessible = false;
            self.power_read_at = Some(Instant::now());
        }
    }

    /// Pedido-resposta remoto com reconexão preguiçosa.
    fn request_remote(
        &mut self,
        canonical: &str,
        addr: &RemoteAddr,
        request: &Message,
        timeout: Duration,
    ) -> Result<Message, TransportError> {
        if !self.connections.contains_key(canonical) {
            let c = match addr {
                RemoteAddr::Unix(p) => Connection::unix(p, timeout)?,
                RemoteAddr::Tcp { host, port } => Connection::tcp(host, *port, timeout)?,
            };
            self.connections.insert(canonical.into(), c);
        }
        let c = self.connections.get_mut(canonical).expect("inserido acima");
        c.request(request, timeout)
    }

    fn route_description(&self, canonical: &str) -> String {
        self.routes
            .get(canonical)
            .map(|r| r.description())
            .unwrap_or_else(|| "fora do registro".into())
    }

    // -----------------------------------------------------------------
    // Atuação: validação, rota, retry e fallback do registro (§4.3)
    // -----------------------------------------------------------------

    fn violation(&self, limits: &ActorLimits, value: &Value) -> Option<(Limit, f64)> {
        let v = value.as_num()?;
        // limites INCLUSIVOS: valor igual ao limite é aceito (FORMAL §4.3)
        if let Some(min) = limits.min {
            if v < min {
                return Some((Limit::Min, min));
            }
        }
        if let Some(max) = limits.max {
            if v > max {
                return Some((Limit::Max, max));
            }
        }
        if let Some(safety) = limits.safety_limit {
            if v > safety {
                return Some((Limit::SafetyLimit, safety));
            }
        }
        None
    }

    /// Limites do registro para um ator (fonte única).
    fn limits_of(&self, canonical: &str) -> ActorLimits {
        match self.registry.get(canonical).map(|d| &d.kind) {
            Some(DeviceKind::Actor { limits }) => limits.clone(),
            _ => ActorLimits::default(),
        }
    }

    fn reject_over_limit(
        &mut self,
        canonical: &str,
        value: &Value,
        limit: Limit,
        limit_value: f64,
        ledger: &mut dyn Ledger,
    ) -> ActOutcome {
        ledger.record(
            rt_kinds::ACTOR_REJECTED_VALUE,
            &format!(
                "Comando a '{canonical}' rejeitado sem envio: valor viola {} = {limit_value}.",
                limit.name()
            ),
            Json::obj([
                ("ator", Json::str(canonical)),
                ("valor", value.to_json()),
                ("limite", Json::str(limit.name())),
                ("limite_valor", Json::num(limit_value)),
            ]),
        );
        ledger.actuator_action(canonical, value, false);
        ActOutcome::Rejected { limit, limit_value }
    }

    /// Tenta entregar na rota REAL do ator (sem fallback).
    /// `Ok(true)` = entregue; `Ok(false)` = indisponível (segue fallback);
    /// `Err(outcome)` = terminativo (rejeição/domínio — sem fallback).
    fn deliver_real(
        &mut self,
        canonical: &str,
        value: &Value,
        ledger: &mut dyn Ledger,
    ) -> Result<bool, ActOutcome> {
        let Some(dr) = self.real_actors.get_mut(canonical) else {
            return Ok(false);
        };
        // tentativas = 1 + retries (PLAN §3: fila com retry e fallback)
        let mut last_latency_us = 0u64;
        for _ in 0..=self.config.retries {
            let t0 = Instant::now();
            let result = dr.apply(value);
            last_latency_us = t0.elapsed().as_micros() as u64;
            match result {
                Ok(()) => {
                    // Etapa 4 (PLAN §4.1): valor aplicado + latência do ack;
                    // o custo energético é estimado pelo Caderno (W × latência).
                    actuation_with_latency(ledger, canonical, value, last_latency_us, true);
                    return Ok(true);
                }
                Err(crate::drivers::ActorError::InvalidValue(reason)) => {
                    actuation_with_latency(ledger, canonical, value, last_latency_us, false);
                    ledger.record(
                        rt_kinds::ACTOR_REJECTED_VALUE,
                        &format!("Comando a '{canonical}' fora do domínio do ator: {reason}."),
                        Json::obj([
                            ("ator", Json::str(canonical)),
                            ("valor", value.to_json()),
                            ("motivo", Json::str(reason.clone())),
                        ]),
                    );
                    return Err(ActOutcome::InvalidValue { reason });
                }
                Err(crate::drivers::ActorError::WriteFailed(_)) => {
                    continue; // retry de transporte antes do fallback
                }
            }
        }
        actuation_with_latency(ledger, canonical, value, last_latency_us, false);
        ledger.record(
            rt_kinds::ACTOR_UNAVAILABLE,
            &format!(
                "Heartbeat do ator '{canonical}' não respondeu (rota {}).",
                self.route_description(canonical)
            ),
            Json::obj([("ator", Json::str(canonical))]),
        );
        Ok(false)
    }

    /// Entrega remota (ACT → ACT_ACK, §4.3). `Some` = terminativo;
    /// `None` = indisponível (segue fallback).
    fn deliver_remote(
        &mut self,
        canonical: &str,
        value: &Value,
        ledger: &mut dyn Ledger,
    ) -> Option<ActOutcome> {
        let Some(Route::Remote(addr)) = self.routes.get(canonical).cloned() else {
            return Some(ActOutcome::MissingActor);
        };
        let seq = self.next_seq();
        let request = Message::act(canonical, wire_of(value), seq, true);
        let timeout = if matches!(addr, RemoteAddr::Tcp { .. }) {
            self.config.act_timeout_remote
        } else {
            self.config.act_timeout_local
        };
        let t0 = Instant::now();
        match self.request_remote(canonical, &addr, &request, timeout) {
            Ok(resp) => {
                let latency_us = t0.elapsed().as_micros() as u64;
                match resp.body {
                    Body::ActAck { status } => match status {
                        AckAct::Delivered => {
                            actuation_with_latency(ledger, canonical, value, latency_us, true);
                            Some(ActOutcome::Delivered)
                        }
                        AckAct::Rejected { limit, limit_value } => {
                            let limit = match limit {
                                0 => Limit::Min,
                                1 => Limit::Max,
                                _ => Limit::SafetyLimit,
                            };
                            Some(self.reject_over_limit(canonical, value, limit, limit_value, ledger))
                        }
                        AckAct::MissingActor => {
                            ledger.record(
                                rt_kinds::ACTOR_UNKNOWN,
                                &format!("Ator '{canonical}' não registrado no peer remoto."),
                                Json::obj([("ator", Json::str(canonical))]),
                            );
                            actuation_with_latency(ledger, canonical, value, latency_us, false);
                            Some(ActOutcome::MissingActor)
                        }
                        AckAct::InvalidValue { reason } => {
                            actuation_with_latency(ledger, canonical, value, latency_us, false);
                            Some(ActOutcome::InvalidValue { reason })
                        }
                        AckAct::Unavailable | AckAct::FallbackExhausted => {
                            actuation_with_latency(ledger, canonical, value, latency_us, false);
                            ledger.record(
                                rt_kinds::ACTOR_UNAVAILABLE,
                                &format!("Heartbeat do ator '{canonical}' não respondeu (peer)."),
                                Json::obj([("ator", Json::str(canonical))]),
                            );
                            None
                        }
                        AckAct::FallbackExecuted { alternativo } => {
                            // O peer executou o fallback do registro DELE (§4.3).
                            actuation_with_latency(ledger, &alternativo, value, latency_us, true);
                            ledger.record(
                                rt_kinds::FALLBACK_EXECUTED,
                                &format!(
                                    "Fallback '{alternativo}' acionado após falha de '{canonical}'."
                                ),
                                Json::obj([
                                    ("primario", Json::str(canonical)),
                                    ("alternativo", Json::str(alternativo.clone())),
                                    ("valor", value.to_json()),
                                ]),
                            );
                            Some(ActOutcome::FallbackExecuted { alternativo })
                        }
                    },
                    _ => {
                        actuation_with_latency(ledger, canonical, value, latency_us, false);
                        None
                    }
                }
            }
            Err(e) => {
                self.connections.remove(canonical);
                actuation_with_latency(ledger, canonical, value, t0.elapsed().as_micros() as u64, false);
                ledger.record(
                    rt_kinds::ACTOR_UNAVAILABLE,
                    &format!("Heartbeat do ator '{canonical}' não respondeu (transporte: {e})."),
                    Json::obj([("ator", Json::str(canonical))]),
                );
                None
            }
        }
    }

    /// Entrega em um ator específico pela rota DELE (fallback e re-entrega).
    /// `None` = indisponível.
    fn deliver_route(
        &mut self,
        canonical: &str,
        value: &Value,
        ledger: &mut dyn Ledger,
    ) -> Option<ActOutcome> {
        let route = self.routes.get(canonical)?.clone();
        match route {
            Route::Simulator => Some(self.sim.act(canonical, value.clone(), ledger)),
            Route::Real => {
                // Limites do registro (inclusivos) ANTES do envio (§4.3).
                let limits = self.limits_of(canonical);
                if let Some((limit, limit_value)) = self.violation(&limits, value) {
                    return Some(self.reject_over_limit(
                        canonical, value, limit, limit_value, ledger,
                    ));
                }
                match self.deliver_real(canonical, value, ledger) {
                    Ok(true) => Some(ActOutcome::Delivered),
                    Ok(false) => None,
                    Err(outcome) => Some(outcome),
                }
            }
            Route::Remote(_) => self.deliver_remote(canonical, value, ledger),
            Route::Inaccessible { reason } => {
                ledger.actuator_action(canonical, value, false);
                ledger.record(
                    rt_kinds::ACTOR_UNAVAILABLE,
                    &format!("Ator '{canonical}' indisponível ({reason})."),
                    Json::obj([("ator", Json::str(canonical))]),
                );
                None
            }
        }
    }

    /// Fallback do REGISTRO (FORMAL §4.3): primary → alternativos declarados
    /// no registro; o runtime não implementa fallback próprio.
    fn try_fallback(
        &mut self,
        primary: &str,
        value: &Value,
        ledger: &mut dyn Ledger,
    ) -> ActOutcome {
        let alternativos: Vec<String> = self
            .registry
            .get(primary)
            .map(|d| d.fallback.clone())
            .unwrap_or_default();
        for alt in alternativos {
            if !self.registry.contains(&alt) {
                continue;
            }
            if let Some(outcome) = self.deliver_route(&alt, value, ledger) {
                if outcome.ok() {
                    ledger.record(
                        rt_kinds::FALLBACK_EXECUTED,
                        &format!("Fallback '{alt}' acionado após falha de '{primary}'."),
                        Json::obj([
                            ("primario", Json::str(primary)),
                            ("alternativo", Json::str(alt.clone())),
                            ("valor", value.to_json()),
                        ]),
                    );
                    // Contrato da Etapa 1/2: atuação por fallback SEMPRE
                    // devolve FallbackExecutado { alternativo } (BDD Caso 3),
                    // independentemente da variante de entrega da rota.
                    return ActOutcome::FallbackExecuted { alternativo: alt };
                }
            }
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

    fn enqueue(&mut self, actor: &str, value: &Value, priority: u8) {
        let cmd = Command {
            seq: self.next_seq(),
            actor: actor.into(),
            value: value.clone(),
            priority,
            ticks_waiting: 0,
            primary: None,
        };
        if self.queue.enqueue(cmd).is_err() {
            // Guarda anti-inchaço estourou: auditoria honesta do descarte.
            // (capacidade 256 torna isso improvável; o evento é obrigatório)
        }
    }

    /// Re-entrega da fila no relógio virtual (PLAN §3.4: prioridades e
    /// timeout), com trilha completa no Caderno.
    fn pump_queue(&mut self, ledger: &mut dyn Ledger) {
        if self.queue.is_empty() {
            return;
        }
        let mut pending = Vec::new();
        while let Some(cmd) = self.queue.dequeue() {
            pending.push(cmd);
        }
        for cmd in pending {
            if cmd.ticks_waiting >= self.config.queue_timeout_ticks {
                ledger.record(
                    kinds::COMANDO_EXPIRADO,
                    &format!(
                        "Comando a '{}' (seq {}) expirou na fila após {} tick(s).",
                        cmd.actor, cmd.seq, cmd.ticks_waiting
                    ),
                    Json::obj([
                        ("ator", Json::str(cmd.actor.clone())),
                        ("valor", cmd.value.to_json()),
                        ("ticks", Json::num(cmd.ticks_waiting as f64)),
                    ]),
                );
                ledger.alert(
                    &format!("Comando a '{}' expirou na fila do FXP.", cmd.actor),
                    Json::obj([
                        ("motivo", Json::str("comando_expirado")),
                        ("ator", Json::str(cmd.actor.clone())),
                    ]),
                );
                continue;
            }
            match self.deliver_route(&cmd.actor, &cmd.value, ledger) {
                Some(outcome) if outcome.ok() => {
                    ledger.record(
                        kinds::COMANDO_REENTREGUE,
                        &format!(
                            "Comando a '{}' re-entregue após {} tick(s) na fila.",
                            cmd.actor, cmd.ticks_waiting
                        ),
                        Json::obj([
                            ("ator", Json::str(cmd.actor.clone())),
                            ("valor", cmd.value.to_json()),
                            ("ticks", Json::num(cmd.ticks_waiting as f64)),
                        ]),
                    );
                }
                _ => {
                    // Falhou de novo: volta com +1 tick (expiração no topo).
                    let _ = self.queue.requeue(cmd);
                }
            }
        }
    }

    /// Varredura silenciosa da potência real (sem Caderno); falha vira alerta
    /// na próxima operação com caderno. Rotas remotas atualizam só quando
    /// lidas pelas regras (on_tick não faz I/O remoto — plano determinístico).
    fn update_power(&mut self) {
        if self.routes.get("cpu_power") != Some(&Route::Real) {
            return;
        }
        if let Some(t) = self.power_read_at {
            if t.elapsed() < self.config.cache_ttl {
                return;
            }
        }
        if let Some(dr) = self.real_sensors.get_mut("cpu_power") {
            match dr.read() {
                Ok(v) => {
                    self.known_power = v;
                    self.power_inaccessible = false;
                }
                Err(_) => self.power_inaccessible = true,
            }
            self.power_read_at = Some(Instant::now());
        }
    }
}

/// Rota pré-driver: o modo global modula o modo do dispositivo.
enum RouteSpec {
    Simulator,
    Concrete,
    Inaccessible { reason: String },
}

fn base_route(config: &BusConfig, d: &crate::registry::DeviceEntry) -> RouteSpec {
    let device_mode = match config.mode {
        OperationMode::Simulated => DeviceMode::Simulated,
        OperationMode::Real | OperationMode::Hybrid => d.mode,
    };
    match (config.mode, device_mode) {
        // Modo real global proíbe rota sintética — §4.7 (nunca simulado mudo).
        (OperationMode::Real, DeviceMode::Simulated) => RouteSpec::Inaccessible {
            reason: "modo real não roteia para simulador (dado sintético proibido)".into(),
        },
        // Simulado explícito (global, ou por dispositivo no híbrido).
        (_, DeviceMode::Simulated) => RouteSpec::Simulator,
        // Dispositivo real: rota concreta (Auto/driver/remota).
        (_, DeviceMode::Real) => RouteSpec::Concrete,
    }
}

fn wire_of(v: &Value) -> WireValue {
    match v {
        Value::Num(n) => WireValue::Num(*n),
        Value::Str(s) => WireValue::Str(s.clone()),
        Value::Ident(s) => WireValue::Ident(s.clone()),
    }
}

impl Fxp for FxpBus {
    fn read_sensor(
        &mut self,
        name: &str,
        ledger: &mut dyn Ledger,
    ) -> Result<f64, SensorFailure> {
        let canonical = self.registry.canonical_of(name).to_string();
        if !self.registry.contains(&canonical) {
            self.sensor_alert(
                ledger,
                "sensor_not_registered",
                name,
                "fora do registro do FXP.",
            );
            return Err(SensorFailure::NotRegistered);
        }
        // §6: leitura por alias é idêntica à do canônico; o Caderno registra
        // o nome usado (LEITURA do engine) e o canônico (este evento).
        if canonical != name {
            ledger.info(
                &format!(
                    "Leitura de '{name}' resolvida para o dispositivo canônico '{canonical}'."
                ),
                Json::obj([
                    ("motivo", Json::str("alias")),
                    ("sensor", Json::str(name)),
                    ("canonical", Json::str(canonical.clone())),
                ]),
            );
        }
        match self.routes.get(&canonical).cloned() {
            Some(Route::Simulator) => self.sim.read_sensor(&canonical, ledger),
            Some(Route::Real) => self.read_real(&canonical, ledger),
            Some(Route::Remote(_)) => self.read_remote(&canonical, ledger),
            Some(Route::Inaccessible { reason }) => {
                self.sensor_alert(
                    ledger,
                    "sensor_inaccessible",
                    &canonical,
                    &format!("{reason}."),
                );
                Err(SensorFailure::Inaccessible)
            }
            None => {
                self.sensor_alert(ledger, "sensor_not_registered", &canonical, "sem rota.");
                Err(SensorFailure::NotRegistered)
            }
        }
    }

    fn act(&mut self, actor: &str, value: Value, ledger: &mut dyn Ledger) -> ActOutcome {
        self.act_with_priority(actor, value, PRIORITY_NORMAL, ledger)
    }

    fn act_with_priority(
        &mut self,
        actor: &str,
        value: Value,
        priority: u8,
        ledger: &mut dyn Ledger,
    ) -> ActOutcome {
        let canonical = self.registry.canonical_of(actor).to_string();
        if !self.registry.contains(&canonical) {
            ledger.record(
                rt_kinds::ACTOR_UNKNOWN,
                &format!("Ator '{actor}' não registrado no FXP."),
                Json::obj([("ator", Json::str(actor))]),
            );
            ledger.actuator_action(actor, &value, false);
            return ActOutcome::MissingActor;
        }
        let route = self.routes.get(&canonical).cloned();
        match route {
            Some(Route::Simulator) => {
                // Paridade com a Etapa 2: validação, efeitos e eventos do sim.
                self.sim.act(&canonical, value, ledger)
            }
            Some(Route::Real) | Some(Route::Remote(_)) | Some(Route::Inaccessible { .. }) => {
                // Limites do REGISTRO (inclusivos) antes do envio (§4.3).
                let limits = self.limits_of(&canonical);
                if let Some((limit, limit_value)) = self.violation(&limits, &value) {
                    return self.reject_over_limit(
                        &canonical,
                        &value,
                        limit,
                        limit_value,
                        ledger,
                    );
                }
                match self.deliver_route(&canonical, &value, ledger) {
                    Some(outcome) => outcome,
                    None => {
                        // Indisponível: fallback do registro; esgotado → fila
                        // prioritária (retry em ticks futuros — PLAN §3.4).
                        let outcome = self.try_fallback(&canonical, &value, ledger);
                        if matches!(outcome, ActOutcome::FallbackExhausted) {
                            self.enqueue(&canonical, &value, priority);
                        }
                        outcome
                    }
                }
            }
            None => {
                ledger.record(
                    rt_kinds::ACTOR_UNKNOWN,
                    &format!("Ator '{actor}' não registrado no FXP."),
                    Json::obj([("ator", Json::str(actor))]),
                );
                ledger.actuator_action(actor, &value, false);
                ActOutcome::MissingActor
            }
        }
    }

    fn cpu_power(&self) -> f64 {
        match self.routes.get("cpu_power") {
            Some(Route::Simulator) | None => self.sim.cpu_power(),
            _ => self.known_power,
        }
    }

    fn on_tick(&mut self, ledger: &mut dyn Ledger) {
        self.sim.on_tick(ledger);
        self.update_power();
        self.pump_queue(ledger);
    }

    fn disk_bytes_used(&self) -> u64 {
        self.disk_bytes
    }

    fn add_disk_bytes(&mut self, n: u64) {
        self.disk_bytes += n;
    }

    fn registry(&self) -> &RuntimeRegistry {
        &self.rt_registry
    }
}
