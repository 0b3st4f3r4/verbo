//! Simulador físico determinístico do FXP (esqueleto da Etapa 1, PLAN §6.5 —
//! evolui para módulo próprio na Etapa 3).
//!
//! - Séries temporais roteirizadas (`programar`/`set_sensor`): determinismo
//!   total, sem aleatoriedade;
//! - Injeção de falhas: sensor ausente, registrado porém inacessível, ator
//!   que não responde (heartbeat);
//! - Registro mínimo obrigatório (FORMAL §6) com limites inclusivos;
//! - Política de fallback no REGISTRO (FORMAL §4.3) — o runtime não implementa
//!   fallback próprio;
//! - Mensagens FXP serializadas em processo (schema binário v1: Etapa 3).

use crate::caderno::{kinds, Caderno};
use crate::fxp::{
    ActOutcome, ActorLimits, ActorState, EfeitoAtor, FalhaSensor, Fxp, Limite, Registry, SensorInfo,
    SensorState, Value,
};
use crate::json::Json;
use std::collections::BTreeMap;

/// Mensagem FXP serializada (fronteira em processo, sem schema binário).
#[derive(Debug, Clone, PartialEq)]
pub struct Mensagem {
    pub seq: u64,
    pub op: String,
    pub ator: String,
    pub valor: Value,
    pub tick: u64,
    /// Presente quando a entrega veio de um fallback.
    pub fallback_de: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SimuladorBuilder {
    registry: Registry,
    valores: BTreeMap<String, f64>,
}

impl Default for SimuladorBuilder {
    fn default() -> Self {
        Self::novo()
    }
}

impl SimuladorBuilder {
    /// Registro mínimo (FORMAL §6) + valores iniciais plausíveis.
    pub fn novo() -> Self {
        Self {
            registry: Registry::minimo(),
            valores: BTreeMap::from([
                ("cpu_temp".to_string(), 55.0),
                ("cpu_power".to_string(), 150.0),
                ("attention".to_string(), 100.0),
            ]),
        }
    }

    pub fn com_sensor(mut self, nome: &str, info: SensorInfo, valor: f64) -> Self {
        self.registry.sensores.insert(nome.into(), info);
        self.valores.insert(nome.into(), valor);
        self
    }

    pub fn com_ator(mut self, nome: &str, limits: ActorLimits) -> Self {
        self.registry.atores.insert(nome.into(), limits);
        self
    }

    pub fn com_valor(mut self, sensor: &str, valor: f64) -> Self {
        self.valores.insert(sensor.into(), valor);
        self
    }

    pub fn construir(self) -> FxpSimulator {
        let mut sim = FxpSimulator {
            registry: self.registry,
            sensores: BTreeMap::new(),
            atores: BTreeMap::new(),
            outbox: Vec::new(),
            entregues: Vec::new(),
            seq: 0,
            ticks: 0,
            cronograma: BTreeMap::new(),
            disk_bytes_used: 1024,
        };
        for (nome, valor) in self.valores {
            sim.sensores
                .insert(nome.clone(), SensorState { valor, acessivel: true });
        }
        // Atores com efeitos físicos determinísticos (PLAN §6.5)
        for nome in ["CpuPowerCap", "Ventoinha", "LedIndicador"] {
            if let Some(limits) = sim.registry.atores.get(nome).cloned() {
                let efeito = match nome {
                    "CpuPowerCap" => EfeitoAtor::PowerCap,
                    "Ventoinha" => EfeitoAtor::Ventoinha,
                    _ => EfeitoAtor::Nenhum,
                };
                sim.atores.insert(
                    nome.to_string(),
                    ActorState { limits, atual: None, disponivel: true, fallback: Vec::new(), efeito },
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
    pub atores: BTreeMap<String, ActorState>,
    /// Mensagens serializadas (todo comando, mesmo rejeitado).
    pub outbox: Vec<Mensagem>,
    /// Entregas efetivadas (ator correto).
    pub entregues: Vec<Mensagem>,
    seq: u64,
    ticks: u64,
    /// Séries roteirizadas: tick (1-based) → valores absolutos de sensores.
    cronograma: BTreeMap<u64, Vec<(String, f64)>>,
    disk_bytes_used: u64,
}

impl FxpSimulator {
    pub fn novo() -> Self {
        SimuladorBuilder::novo().construir()
    }

    // ------------------------------------------------------------------
    // Roteirização do mundo e injeção de falhas (PLAN §6.5)
    // ------------------------------------------------------------------
    pub fn set_sensor(&mut self, nome: &str, valor: f64) {
        if let Some(s) = self.sensores.get_mut(nome) {
            s.valor = valor;
            s.acessivel = true;
        }
    }

    /// Sensor registrado porém inacessível (FORMAL §4.7).
    pub fn falhar_sensor(&mut self, nome: &str) {
        if let Some(s) = self.sensores.get_mut(nome) {
            s.acessivel = false;
        }
    }

    pub fn recuperar_sensor(&mut self, nome: &str) {
        if let Some(s) = self.sensores.get_mut(nome) {
            s.acessivel = true;
        }
    }

    /// Remove sensor do registro (falha `sensor_nao_registrado`).
    pub fn desregistrar_sensor(&mut self, nome: &str) {
        self.sensores.remove(nome);
        self.registry.sensores.remove(nome);
    }

    /// Agenda valor absoluto para um tick (1-based).
    pub fn programar(&mut self, tick: u64, sensor: &str, valor: f64) {
        self.cronograma.entry(tick).or_default().push((sensor.into(), valor));
    }

    /// Ator para de responder (heartbeat falho — BDD Caso 3).
    pub fn falhar_ator(&mut self, nome: &str) {
        if let Some(a) = self.atores.get_mut(nome) {
            a.disponivel = false;
        }
    }

    pub fn recuperar_ator(&mut self, nome: &str) {
        if let Some(a) = self.atores.get_mut(nome) {
            a.disponivel = true;
        }
    }

    /// Política de fallback fica no REGISTRO do FXP (FORMAL §4.3).
    pub fn definir_fallback(&mut self, primario: &str, alternativos: &[&str]) {
        if let Some(a) = self.atores.get_mut(primario) {
            a.fallback = alternativos.iter().map(|s| s.to_string()).collect();
        }
    }

    /// Registra ator extra (extensão opcional, ex.: `VentoinhaReserva`).
    pub fn registrar_ator(&mut self, nome: &str, limits: ActorLimits) {
        self.registry.atores.insert(nome.into(), limits.clone());
        self.atores.insert(
            nome.into(),
            ActorState { limits, atual: None, disponivel: true, fallback: Vec::new(), efeito: EfeitoAtor::Nenhum },
        );
    }

    /// Registra (ou substitui) um sensor no registro/simulador — usado pelo
    /// bus da Etapa 3 para sincronizar o `DeviceRegistry` (fonte única) com
    /// o backend simulado; valor inicial plausível é 0.0 até roteirização.
    pub fn registrar_sensor(&mut self, nome: &str, info: SensorInfo) {
        self.registry.sensores.insert(nome.into(), info);
        self.sensores.insert(
            nome.into(),
            SensorState { valor: 0.0, acessivel: true },
        );
    }

    // ------------------------------------------------------------------
    // Observação (testes/CLI)
    // ------------------------------------------------------------------
    pub fn ator_atual(&self, nome: &str) -> Option<&Value> {
        self.atores.get(nome).and_then(|a| a.atual.as_ref())
    }

    /// Registro atual (sensores/atores + limites) — FORMAL §6.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn sensor_valor(&self, nome: &str) -> Option<f64> {
        self.sensores.get(nome).map(|s| s.valor)
    }

    fn violacao_de_limite(limits: &ActorLimits, valor: f64) -> Option<(Limite, f64)> {
        // limites INCLUSIVOS: valor igual ao limite é aceito (FORMAL §4.3)
        if let Some(min) = limits.min {
            if valor < min {
                return Some((Limite::Min, min));
            }
        }
        if let Some(max) = limits.max {
            if valor > max {
                return Some((Limite::Max, max));
            }
        }
        if let Some(safety) = limits.safety_limit {
            if valor > safety {
                return Some((Limite::SafetyLimit, safety));
            }
        }
        None
    }

    fn entregar(&mut self, ator: &str, valor: &Value, msg: Mensagem, caderno: &mut dyn Caderno) {
        let efeito = self.atores.get(ator).map(|a| a.efeito).unwrap_or_default();
        match efeito {
            EfeitoAtor::PowerCap => {
                if let Some(v) = valor.as_num() {
                    let atual = self.sensor_valor("cpu_power").unwrap_or(v);
                    if v < atual {
                        self.set_sensor("cpu_power", v);
                    }
                }
            }
            EfeitoAtor::Ventoinha => {
                if let Some(v) = valor.as_num() {
                    if let Some(t) = self.sensor_valor("cpu_temp") {
                        let novo = (t - (v / 255.0) * 8.0).max(0.0);
                        self.set_sensor("cpu_temp", novo);
                    }
                }
            }
            EfeitoAtor::Nenhum => {}
        }
        if let Some(a) = self.atores.get_mut(ator) {
            a.atual = Some(valor.clone());
        }
        self.entregues.push(msg);
        caderno.actuator_action(ator, valor, true);
    }
}

impl Default for FxpSimulator {
    fn default() -> Self {
        Self::novo()
    }
}

impl Fxp for FxpSimulator {
    fn read_sensor(
        &mut self,
        nome: &str,
        caderno: &mut dyn Caderno,
    ) -> Result<f64, FalhaSensor> {
        let sensor = self.sensores.get(nome);
        match sensor {
            None => {
                caderno.alert(
                    &format!(
                        "Sensor '{nome}' não registrado no FXP (falha de I/O). Condição não avaliada neste tick."
                    ),
                    Json::obj([
                        ("motivo", Json::str("sensor_nao_registrado")),
                        ("sensor", Json::str(nome)),
                    ]),
                );
                Err(FalhaSensor::NaoRegistrado)
            }
            Some(s) if !s.acessivel => {
                caderno.alert(
                    &format!(
                        "Sensor '{nome}' registrado porém inacessível (falha de leitura). Condição não avaliada neste tick."
                    ),
                    Json::obj([
                        ("motivo", Json::str("sensor_inacessivel")),
                        ("sensor", Json::str(nome)),
                    ]),
                );
                Err(FalhaSensor::Inacessivel)
            }
            Some(s) => Ok(arredondar2(s.valor)),
        }
    }

    fn act(&mut self, ator: &str, valor: Value, caderno: &mut dyn Caderno) -> ActOutcome {
        self.seq += 1;
        let msg = Mensagem {
            seq: self.seq,
            op: "act".into(),
            ator: ator.into(),
            valor: valor.clone(),
            tick: self.ticks,
            fallback_de: None,
        };
        self.outbox.push(msg);

        let Some(estado) = self.atores.get(ator).cloned() else {
            caderno.record(
                kinds::ATOR_INEXISTENTE,
                &format!("Ator '{ator}' não registrado no FXP."),
                Json::obj([("ator", Json::str(ator))]),
            );
            caderno.actuator_action(ator, &valor, false);
            return ActOutcome::AtorInexistente;
        };

        if !estado.disponivel {
            caderno.actuator_action(ator, &valor, false);
            caderno.record(
                kinds::ATOR_INDISPONIVEL,
                &format!("Heartbeat do ator '{ator}' não respondeu."),
                Json::obj([("ator", Json::str(ator))]),
            );
            return self.tentar_fallback(ator, &valor, &estado.fallback, caderno);
        }

        if let Some(v) = valor.as_num() {
            if let Some((limite, valor_limite)) = Self::violacao_de_limite(&estado.limits, v) {
                caderno.record(
                    kinds::ACTOR_REJECTED_VALUE,
                    &format!(
                        "Comando a '{ator}' rejeitado sem envio: valor {v} viola {} = {valor_limite}.",
                        limite.nome()
                    ),
                    Json::obj([
                        ("ator", Json::str(ator)),
                        ("valor", Json::num(v)),
                        ("limite", Json::str(limite.nome())),
                        ("limite_valor", Json::num(valor_limite)),
                    ]),
                );
                caderno.actuator_action(ator, &valor, false);
                return ActOutcome::Rejeitado { limite, valor_limite };
            }
        }

        let msg = self.outbox.last().unwrap().clone();
        self.entregar(ator, &valor, msg, caderno);
        ActOutcome::Entregue
    }

    fn cpu_power(&self) -> f64 {
        self.sensores.get("cpu_power").map(|s| s.valor).unwrap_or(0.0)
    }

    fn on_tick(&mut self, _caderno: &mut dyn Caderno) {
        self.ticks += 1;
        if let Some(valores) = self.cronograma.remove(&self.ticks) {
            for (nome, valor) in valores {
                self.set_sensor(&nome, valor);
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
    fn tentar_fallback(
        &mut self,
        primario: &str,
        valor: &Value,
        fallback: &[String],
        caderno: &mut dyn Caderno,
    ) -> ActOutcome {
        for alt in fallback {
            let Some(alt_limits) = self.registry.atores.get(alt).cloned() else {
                continue;
            };
            let disponivel = self.atores.get(alt).map(|a| a.disponivel).unwrap_or(false);
            if !disponivel {
                continue;
            }
            if let Some(v) = valor.as_num() {
                if Self::violacao_de_limite(&alt_limits, v).is_some() {
                    caderno.alert(
                        &format!("Fallback '{alt}' rejeitou o valor {v} (limites)."),
                        Json::obj([
                            ("motivo", Json::str("fallback_rejeitado")),
                            ("ator", Json::str(alt)),
                        ]),
                    );
                    continue;
                }
            }
            let msg = Mensagem {
                seq: self.seq,
                op: "act".into(),
                ator: alt.clone(),
                valor: valor.clone(),
                tick: self.ticks,
                fallback_de: Some(primario.into()),
            };
            self.outbox.push(msg.clone());
            self.entregar(alt, valor, msg, caderno);
            caderno.record(
                kinds::FALLBACK_EXECUTADO,
                &format!("Fallback '{alt}' acionado após falha de '{primario}'."),
                Json::obj([
                    ("primario", Json::str(primario)),
                    ("alternativo", Json::str(alt)),
                    ("valor", valor.to_json()),
                ]),
            );
            return ActOutcome::FallbackExecutado { alternativo: alt.clone() };
        }
        caderno.alert(
            &format!("Todos os fallbacks de '{primario}' falharam."),
            Json::obj([
                ("motivo", Json::str("fallback_esgotado")),
                ("ator", Json::str(primario)),
            ]),
        );
        ActOutcome::FallbackEsgotado
    }
}

/// Arredonda para 2 casas (mesma fidelidade do protótipo da Etapa 1).
fn arredondar2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
