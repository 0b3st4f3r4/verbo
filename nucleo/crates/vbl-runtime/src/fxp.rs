//! FXP (Flux Protocol) — barramento de I/O que unifica sensores (entrada) e
//! atores (saída) (FORMAL §4.4/§6).
//!
//! O runtime consome apenas o trait [`Fxp`]: leitura de sensor por nome
//! simbólico e comando a ator — ambos **não bloqueantes** na fronteira
//! (em processo nesta etapa; transporte e schema binário v1: Etapa 3,
//! PLAN §3.5). O registro mínimo obrigatório (FORMAL §6) já vem pronto no
//! simulador, e a política de fallback é do REGISTRO do FXP (FORMAL §4.3).

use crate::caderno::Caderno;
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
    pub grandeza: String,
    pub unidade: String,
}

/// Limites de um ator no registro (FORMAL §6) — validação **inclusiva**.
#[derive(Debug, Clone, Default)]
pub struct ActorLimits {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub safety_limit: Option<f64>,
}

/// Registro do FXP: o que está disponível para leitura e atuação.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub sensores: BTreeMap<String, SensorInfo>,
    pub atores: BTreeMap<String, ActorLimits>,
}

impl Registry {
    /// Registro mínimo obrigatório (FORMAL §6).
    pub fn minimo() -> Self {
        let mut r = Self::default();
        for (nome, grandeza, unidade) in [
            ("cpu_temp", "temperatura", "°C"),
            ("cpu_power", "potencia", "W"),
            ("attention", "atencao", "%"),
        ] {
            r.sensores
                .insert(nome.into(), SensorInfo { grandeza: grandeza.into(), unidade: unidade.into() });
        }
        for (nome, min, max, safety) in [
            ("CpuPowerCap", Some(10.0), Some(250.0), Some(200.0)),
            ("Ventoinha", Some(0.0), Some(255.0), Some(200.0)),
            ("LedIndicador", None, None, None),
        ] {
            r.atores.insert(nome.into(), ActorLimits { min, max, safety_limit: safety });
        }
        r
    }
}

/// Estado de um sensor no simulador.
#[derive(Debug, Clone)]
pub struct SensorState {
    pub valor: f64,
    pub acessivel: bool,
}

/// Estado de um ator no simulador.
#[derive(Debug, Clone)]
pub struct ActorState {
    pub limits: ActorLimits,
    pub atual: Option<Value>,
    pub disponivel: bool,
    /// Política de fallback do registro (FORMAL §4.3): primário → alternativos.
    pub fallback: Vec<String>,
    /// Efeito físico simulado do ator.
    pub efeito: EfeitoAtor,
}

/// Efeitos físicos determinísticos dos atores simulados (PLAN §6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EfeitoAtor {
    /// `CpuPowerCap`: limita a potência (se menor que a atual).
    PowerCap,
    /// `Ventoinha`: resfria a temperatura proporcional ao PWM.
    #[default]
    Nenhum,
    Ventoinha,
}

/// Falha de leitura de sensor (FORMAL §4.7) — nunca é `0.0`.
#[derive(Debug, Clone, PartialEq)]
pub enum FalhaSensor {
    /// Sensor fora do registro do FXP.
    NaoRegistrado,
    /// Registrado porém inacessível (falha de leitura em modo real).
    Inacessivel,
}

impl FalhaSensor {
    /// `motivo` canônico do alerta no Caderno (suíte da Etapa 1).
    pub fn motivo(&self) -> &'static str {
        match self {
            FalhaSensor::NaoRegistrado => "sensor_nao_registrado",
            FalhaSensor::Inacessivel => "sensor_inacessivel",
        }
    }
}

/// Resultado de um comando `act` (FORMAL §4.3).
#[derive(Debug, Clone, PartialEq)]
pub enum ActOutcome {
    Entregue,
    /// Rejeitado sem envio — limite violado (inclusivo: igual ao limite passa).
    Rejeitado { limite: Limite, valor_limite: f64 },
    /// Ator fora do registro.
    AtorInexistente,
    /// Heartbeat do ator não respondeu; fallback tentado conforme registro.
    Indisponivel,
    /// Fallback acionado com sucesso no ator alternativo.
    FallbackExecutado { alternativo: String },
    /// Todos os fallbacks falharam.
    FallbackEsgotado,
}

impl ActOutcome {
    pub fn ok(&self) -> bool {
        matches!(self, ActOutcome::Entregue | ActOutcome::FallbackExecutado { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limite {
    Min,
    Max,
    SafetyLimit,
}

impl Limite {
    pub fn nome(&self) -> &'static str {
        match self {
            Limite::Min => "min",
            Limite::Max => "max",
            Limite::SafetyLimit => "safety_limit",
        }
    }
}

/// Barramento FXP consumido pelo runtime (não bloqueante; timeout na Etapa 3).
pub trait Fxp {
    /// Leitura por nome simbólico. Falha → [`FalhaSensor`] + alerta no Caderno
    /// (§4.7: sensor ausente NUNCA é 0.0 — zero é leitura física válida).
    fn read_sensor(&mut self, nome: &str, caderno: &mut dyn Caderno)
        -> Result<f64, FalhaSensor>;

    /// Comando a ator: serializa, valida limites (inclusivos) e entrega;
    /// o Caderno registra tentativa, falha e fallback (§4.3).
    fn act(&mut self, ator: &str, valor: Value, caderno: &mut dyn Caderno) -> ActOutcome;

    /// Potência global do tick (partilha P/N — FORMAL §4.2).
    fn cpu_power(&self) -> f64;

    /// Avanço do tick no mundo (roteirização do simulador).
    fn on_tick(&mut self);

    /// Bytes em suporte estável (persistência — FORMAL §4.1).
    fn disk_bytes_used(&self) -> u64;
    fn add_disk_bytes(&mut self, n: u64);

    /// Registro (para validação de referências em tempo de carga).
    fn registry(&self) -> &Registry;
}
