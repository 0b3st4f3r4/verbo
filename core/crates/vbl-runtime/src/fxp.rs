//! FXP (Flux Protocol) — barramento de I/O que unifica sensores (entrada) e
//! atores (saída) (FORMAL §4.4/§6).
//!
//! O runtime consome apenas o trait [`Fxp`]: leitura de sensor por nome
//! simbólico e comando a ator — ambos **não bloqueantes** na fronteira
//! (em processo nesta etapa; transporte e schema binário v1: Etapa 3,
//! PLAN §3.5). O registro mínimo obrigatório (FORMAL §6) já vem pronto no
//! simulador, e a política de fallback é do REGISTRO do FXP (FORMAL §4.3).

use crate::ledger::Ledger;
use crate::json::Json;
use std::collections::BTreeMap;

/// Valor de comando/expressão do ator: numérico (limites) ou texto
/// (ex.: `LedIndicador`).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
    Ident(String),
}

impl Value {
    /// Valor numérico, quando aplicável (validação de limites).
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Json {
        match self {
            Value::Num(n) => Json::num(*n),
            Value::Str(s) | Value::Ident(s) => Json::str(s),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Num(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "\"{s}\""),
            Value::Ident(s) => write!(f, "{s}"),
        }
    }
}

/// Grandeza/unidade declarada de um sensor no registro (FORMAL §6).
#[derive(Debug, Clone, PartialEq)]
pub struct SensorInfo {
    pub quantity: String,
    pub unit: String,
}

/// Limites de um ator no registro (FORMAL §6) — validação **inclusiva**.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActorLimits {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub safety_limit: Option<f64>,
}

/// Registro do FXP: o que está disponível para leitura e atuação.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub sensores: BTreeMap<String, SensorInfo>,
    pub actors: BTreeMap<String, ActorLimits>,
}

impl Registry {
    /// Registro mínimo obrigatório (FORMAL §6).
    pub fn minimum() -> Self {
        let mut r = Self::default();
        for (name, quantity, unit) in [
            ("cpu_temp", "temperatura", "°C"),
            ("cpu_power", "potencia", "W"),
            ("attention", "atencao", "%"),
        ] {
            r.sensores
                .insert(name.into(), SensorInfo { quantity: quantity.into(), unit: unit.into() });
        }
        for (name, min, max, safety) in [
            ("CpuPowerCap", Some(10.0), Some(250.0), Some(200.0)),
            ("Ventoinha", Some(0.0), Some(255.0), Some(200.0)),
            ("LedIndicador", None, None, None),
        ] {
            r.actors.insert(name.into(), ActorLimits { min, max, safety_limit: safety });
        }
        r
    }
}

/// Estado de um sensor no simulador.
#[derive(Debug, Clone)]
pub struct SensorState {
    pub value: f64,
    pub accessible: bool,
}

/// Estado de um ator no simulador.
#[derive(Debug, Clone)]
pub struct ActorState {
    pub limits: ActorLimits,
    pub current: Option<Value>,
    pub available: bool,
    /// Política de fallback do registro (FORMAL §4.3): primário → alternativos.
    pub fallback: Vec<String>,
    /// Efeito físico simulado do ator.
    pub effect: ActorEffect,
}

/// Efeitos físicos determinísticos dos atores simulados (PLAN §6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActorEffect {
    /// `CpuPowerCap`: limita a potência (se menor que a atual).
    PowerCap,
    /// `Fan`: resfria a temperatura proporcional ao PWM.
    #[default]
    NoEffect,
    Fan,
}

/// Falha de leitura de sensor (FORMAL §4.7) — nunca é `0.0`.
#[derive(Debug, Clone, PartialEq)]
pub enum SensorFailure {
    /// Sensor fora do registro do FXP.
    NotRegistered,
    /// Registrado porém inacessível (falha de leitura em modo real).
    Inaccessible,
}

impl SensorFailure {
    /// `reason` canônico do alerta no Caderno (suíte da Etapa 1).
    pub fn reason(&self) -> &'static str {
        match self {
            SensorFailure::NotRegistered => "sensor_nao_registrado",
            SensorFailure::Inaccessible => "sensor_inacessivel",
        }
    }
}

/// Resultado de um comando `act` (FORMAL §4.3).
#[derive(Debug, Clone, PartialEq)]
pub enum ActOutcome {
    Delivered,
    /// Rejeitado sem envio — limite violado (inclusivo: igual ao limite passa).
    Rejected { limit: Limit, limit_value: f64 },
    /// Ator fora do registro.
    MissingActor,
    /// Heartbeat do ator não respondeu; fallback tentado conforme registro.
    Unavailable,
    /// Fallback acionado com sucesso no ator alternativo.
    FallbackExecuted { alternativo: String },
    /// Todos os fallbacks falharam.
    FallbackExhausted,
    /// Valor fora do **domínio** do ator (extensão da Etapa 3): limites
    /// numéricos do registro cobrem min/max/safety; domínio cobre o resto
    /// (ex.: cor fora do mapa do `LedIndicador`, texto em ator numérico).
    /// Sempre auditado (`ACT_ACK.InvalidValue` no schema v1), nunca entrega
    /// silenciosa.
    InvalidValue { reason: String },
}

impl ActOutcome {
    pub fn ok(&self) -> bool {
        matches!(
            self,
            ActOutcome::Delivered | ActOutcome::FallbackExecuted { .. }
        )
    }
}

/// Prioridade canônica de comandos na fila do FXP (PLAN §3.4).
/// Máxima para atuações associadas a `subvert` (FORMAL §4.5).
pub const PRIORITY_SUBVERT: u8 = 0;
/// Prioridade padrão de comandos de atuação.
pub const PRIORITY_NORMAL: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Min,
    Max,
    SafetyLimit,
}

impl Limit {
    pub fn name(&self) -> &'static str {
        match self {
            Limit::Min => "min",
            Limit::Max => "max",
            Limit::SafetyLimit => "safety_limit",
        }
    }
}

/// Barramento FXP consumido pelo runtime (não bloqueante; timeout na Etapa 3).
pub trait Fxp {
    /// Leitura por nome simbólico. Falha → [`SensorFailure`] + alerta no Caderno
    /// (§4.7: sensor ausente NUNCA é 0.0 — zero é leitura física válida).
    fn read_sensor(&mut self, name: &str, ledger: &mut dyn Ledger)
        -> Result<f64, SensorFailure>;

    /// Comando a ator: serializa, valida limites (inclusivos) e entrega;
    /// o Caderno registra tentativa, falha e fallback (§4.3).
    fn act(&mut self, actor: &str, value: Value, ledger: &mut dyn Ledger) -> ActOutcome;

    /// Comando com prioridade de fila (Etapa 3): default ignora a prioridade
    /// e delega a [`Fxp::act`] — barramentos com fila prioritária (vbl-fxp)
    /// sobrescrevem. O engine usa [`PRIORITY_SUBVERT`] para a `act` que
    /// segue um `subvert` na mesma regra (FORMAL §4.5: sem atraso
    /// perceptível).
    fn act_with_priority(
        &mut self,
        actor: &str,
        value: Value,
        priority: u8,
        ledger: &mut dyn Ledger,
    ) -> ActOutcome {
        let _ = priority;
        self.act(actor, value, ledger)
    }

    /// Potência global do tick (partilha P/N — FORMAL §4.2).
    fn cpu_power(&self) -> f64;

    /// Avanço do tick no mundo (roteirização do simulador; Etapa 3: também
    /// re-entrega a fila de comandos, auditada no Caderno).
    fn on_tick(&mut self, ledger: &mut dyn Ledger);

    /// Bytes em suporte estável (persistência — FORMAL §4.1).
    fn disk_bytes_used(&self) -> u64;
    fn add_disk_bytes(&mut self, n: u64);

    /// Registro (para validação de referências em tempo de carga).
    fn registry(&self) -> &Registry;
}
