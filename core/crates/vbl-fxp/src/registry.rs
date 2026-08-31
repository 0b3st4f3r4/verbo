//! Registro do FXP — nomes simbólicos → endpoints concretos (FORMAL §6).
//!
//! Evolução do registro da Etapa 2 (`vbl_runtime::fxp::Registry`): além de
//! grandeza/unidade e limites, cada entrada carrega **modo** (real/simulado),
//! **endpoint** concreto, **aliases** (§6: `attention` → `human_attention`;
//! leitura por alias idêntica à do canônico) e política de **fallback**
//! (§4.3: primary → dispositivos alternativos do registro — o runtime não
//! implementa fallback próprio).
//!
//! [`DeviceRegistry::to_runtime_registry`] projeta o registro rico no formato
//! plano consumido pelo loader/runtime — incluindo os aliases, para que a
//! validação de referências aceite ambos os nomes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use vbl_runtime::fxp::{ActorLimits, Registry as RuntimeRegistry, SensorInfo};

/// Modo de operação global do barramento (PLAN §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OperationMode {
    /// Todo dispositivo no endpoint físico; indisponível ⇒ falha honesta.
    Real,
    /// Todo dispositivo sintético, marcado no Caderno (CI-safe; default).
    #[default]
    Simulated,
    /// Por dispositivo: reais onde há endpoint, simulados no restante.
    Hybrid,
}

impl OperationMode {
    pub fn parse(s: &str) -> Result<Self, RegistryError> {
        match s {
            "real" => Ok(Self::Real),
            "simulado" => Ok(Self::Simulated),
            "hibrido" | "híbrido" => Ok(Self::Hybrid),
            other => Err(RegistryError::InvalidConfig(format!(
                "modo desconhecido: '{other}' (use real | simulado | hibrido)"
            ))),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            OperationMode::Real => "real",
            OperationMode::Simulated => "simulado",
            OperationMode::Hybrid => "hibrido",
        }
    }
}

/// Modo do dispositivo individual (o global modula: em `Simulated` puro tudo
/// é sintético; em `Real` dispositivo sem endpoint real é inacessível; em
/// `Hybrid` vale o modo individual).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceMode {
    Real,
    #[default]
    Simulated,
}

impl DeviceMode {
    pub fn parse(s: &str) -> Result<Self, RegistryError> {
        match s {
            "real" => Ok(Self::Real),
            "simulado" => Ok(Self::Simulated),
            other => Err(RegistryError::InvalidConfig(format!(
                "modo de dispositivo desconhecido: '{other}'"
            ))),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            DeviceMode::Real => "real",
            DeviceMode::Simulated => "simulado",
        }
    }
}

/// Grandeza/limites declarados do dispositivo (FORMAL §6).
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceKind {
    Sensor {
        quantity: String,
        unit: String,
        /// Faixa física esperada (documentativa; leitura fora dela é alerta,
        /// nunca fabricação — FORMAL §4.7).
        range: (Option<f64>, Option<f64>),
        /// Precisão típica em percentual (§6; 0.0 = dependente do backend).
        precision_pct: f64,
    },
    Actor {
        limits: ActorLimits,
    },
}

/// Endereço de transporte remoto (schema v1 sobre stream).
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteAddr {
    Unix(PathBuf),
    Tcp { host: String, port: u16 },
}

/// Endpoint concreto do dispositivo — o **nome simbólico nunca é caminho de
/// SO** na linguagem; o mapeamento é daqui (FORMAL §3/§6).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Endpoint {
    /// Backend simulado em processo (determinístico; roteirizado pelo CLI).
    #[default]
    Simulated,
    /// Auto-descoberta no host (só para os dispositivos obrigatórios; falha
    /// de descoberta ⇒ dispositivo registrado porém inacessível — §4.7).
    Auto,
    /// thermal_zone: diretório com `temp` (mili°C) e `type`.
    ThermalZone { dir: PathBuf },
    /// hwmon temperatura: arquivo `tempN_input` (mili°C) — ex.: k10temp.
    HwmonTemp { file: PathBuf },
    /// RAPL consumo: diretório com `energy_uj` (+ `max_energy_range_uj` p/ wrap).
    RaplEnergy { dir: PathBuf },
    /// RAPL power cap: arquivo `constraint_0_power_limit_uw` (µW).
    RaplConstraint { file: PathBuf },
    /// hwmon PWM: arquivo `pwmN` (0–255).
    HwmonPwm { file: PathBuf },
    /// LED class: diretório com `brightness` e `max_brightness`.
    LedClass { dir: PathBuf },
    /// Peer remoto falando schema v1 (`unix:/path` ou `tcp:host:porta`).
    Remote { addr: RemoteAddr },
}

impl Endpoint {
    /// Formato textual do config: `simulado` | `auto` |
    /// `thermal_zone:<dir>` | `hwmon_temp:<arq>` | `rapl_energy:<dir>` |
    /// `rapl_constraint:<arq>` | `hwmon_pwm:<arq>` | `led:<dir>` |
    /// `unix:<path>` | `tcp:<host>:<porta>`.
    pub fn parse(s: &str) -> Result<Self, RegistryError> {
        // Esquemas sem caminho.
        match s {
            "simulado" => return Ok(Endpoint::Simulated),
            "auto" => return Ok(Endpoint::Auto),
            _ => {}
        }
        let (schema, rest) = s
            .split_once(':')
            .ok_or_else(|| RegistryError::InvalidEndpoint(s.into()))?;
        let path = |v: &str| PathBuf::from(v);
        match schema {
            "thermal_zone" => Ok(Endpoint::ThermalZone { dir: path(rest) }),
            "hwmon_temp" => Ok(Endpoint::HwmonTemp { file: path(rest) }),
            "rapl_energy" => Ok(Endpoint::RaplEnergy { dir: path(rest) }),
            "rapl_constraint" => Ok(Endpoint::RaplConstraint { file: path(rest) }),
            "hwmon_pwm" => Ok(Endpoint::HwmonPwm { file: path(rest) }),
            "led" => Ok(Endpoint::LedClass { dir: path(rest) }),
            "unix" => Ok(Endpoint::Remote { addr: RemoteAddr::Unix(path(rest)) }),
            "tcp" => {
                let (host, port) = rest.rsplit_once(':').ok_or_else(|| {
                    RegistryError::InvalidEndpoint(format!("{s} (esperado tcp:host:porta)"))
                })?;
                let port: u16 = port.parse().map_err(|_| {
                    RegistryError::InvalidEndpoint(format!("{s} (porta inválida)"))
                })?;
                Ok(Endpoint::Remote { addr: RemoteAddr::Tcp { host: host.into(), port } })
            }
            _ => Err(RegistryError::InvalidEndpoint(s.into())),
        }
    }

    /// Descrição canônica (re-parseável por [`Endpoint::parse`]; usada no
    /// Caderno, no `vbl fxp-probe` e na documentação).
    pub fn description(&self) -> String {
        match self {
            Endpoint::Simulated => "simulado".into(),
            Endpoint::Auto => "auto".into(),
            Endpoint::ThermalZone { dir } => format!("thermal_zone:{}", dir.display()),
            Endpoint::HwmonTemp { file } => format!("hwmon_temp:{}", file.display()),
            Endpoint::RaplEnergy { dir } => format!("rapl_energy:{}", dir.display()),
            Endpoint::RaplConstraint { file } => format!("rapl_constraint:{}", file.display()),
            Endpoint::HwmonPwm { file } => format!("hwmon_pwm:{}", file.display()),
            Endpoint::LedClass { dir } => format!("led:{}", dir.display()),
            Endpoint::Remote { addr } => match addr {
                RemoteAddr::Unix(p) => format!("unix:{}", p.display()),
                RemoteAddr::Tcp { host, port } => format!("tcp:{host}:{port}"),
            },
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Endpoint::Remote { .. })
    }
}

/// Entrada do registro: canônico único + aliases + modo + rota + fallback.
#[derive(Debug, Clone)]
pub struct DeviceEntry {
    /// Nome canônico único (FORMAL §6).
    pub name: String,
    pub kind: DeviceKind,
    pub mode: DeviceMode,
    pub endpoint: Endpoint,
    /// Dispositivos alternativos em ordem de tentativa (FORMAL §4.3).
    pub fallback: Vec<String>,
    /// Apelidos aceitos na leitura/validação (ex.: `human_attention`).
    pub aliases: Vec<String>,
}

impl DeviceEntry {
    /// Sensor simulado com metadados do §6.
    pub fn sensor(name: &str, quantity: &str, unit: &str, precision_pct: f64) -> Self {
        Self {
            name: name.into(),
            kind: DeviceKind::Sensor {
                quantity: quantity.into(),
                unit: unit.into(),
                range: (None, None),
                precision_pct,
            },
            mode: DeviceMode::Simulated,
            endpoint: Endpoint::Simulated,
            fallback: Vec::new(),
            aliases: Vec::new(),
        }
    }

    /// Ator simulado com limites do §6.
    pub fn actor(name: &str, limits: ActorLimits) -> Self {
        Self {
            name: name.into(),
            kind: DeviceKind::Actor { limits },
            mode: DeviceMode::Simulated,
            endpoint: Endpoint::Simulated,
            fallback: Vec::new(),
            aliases: Vec::new(),
        }
    }
}

/// Erro de registro/configuração.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Nome canônico já ocupado por outro dispositivo.
    DuplicateName(String),
    /// Alias/nome colide com nome canônico ou alias existente.
    ConflictingAlias(String),
    /// Alias aponta para dispositivo inexistente.
    UnknownAlias { alias: String, canonical: String },
    /// Fallback cita alternativo fora do registro (FORMAL §4.3).
    UnknownFallback { actor: String, alternativo: String },
    /// Alias definido sobre outro alias (encadeamento não é permitido).
    ChainedAlias(String),
    /// String de endpoint fora do formato.
    InvalidEndpoint(String),
    /// Config malformada (linha, valor, dispositivo subespecificado…).
    InvalidConfig(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::DuplicateName(n) => {
                write!(f, "nome canônico '{n}' já registrado")
            }
            RegistryError::ConflictingAlias(n) => {
                write!(f, "'{n}' colide com nome/alias já registrado")
            }
            RegistryError::UnknownAlias { alias, canonical } => {
                write!(f, "alias '{alias}' aponta para dispositivo inexistente '{canonical}'")
            }
            RegistryError::UnknownFallback { actor, alternativo } => {
                write!(f, "fallback de '{actor}' cita '{alternativo}', fora do registro (FORMAL §4.3)")
            }
            RegistryError::ChainedAlias(n) => {
                write!(f, "alias '{n}' não pode apontar para outro alias")
            }
            RegistryError::InvalidEndpoint(e) => {
                write!(f, "endpoint inválido: '{e}' (simulado | auto | thermal_zone:… | rapl_energy:… | rapl_constraint:… | hwmon_pwm:… | led:… | unix:… | tcp:host:porta)")
            }
            RegistryError::InvalidConfig(m) => write!(f, "config inválida: {m}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Registro do FXP: mapeamento nome simbólico → dispositivo concreto.
#[derive(Debug, Clone, Default)]
pub struct DeviceRegistry {
    devices: BTreeMap<String, DeviceEntry>,
    /// alias → canônico (resolução O(1); FORMAL §6).
    aliases: BTreeMap<String, String>,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registro mínimo obrigatório (FORMAL §6) — modo simulado (CI-safe).
    /// Precisões típicas: cpu_temp ±2%, cpu_power ±5%, attention dependente
    /// do backend (0.0 = não declarado).
    pub fn minimum() -> Self {
        let mut r = Self::new();
        for (name, quantity, unit, precision) in [
            ("cpu_temp", "temperatura", "°C", 2.0),
            ("cpu_power", "potencia", "W", 5.0),
            ("attention", "atencao", "%", 0.0),
        ] {
            let _ = r.register(DeviceEntry::sensor(name, quantity, unit, precision));
        }
        for (name, min, max, safety) in [
            ("CpuPowerCap", Some(10.0), Some(250.0), Some(200.0)),
            ("Ventoinha", Some(0.0), Some(255.0), Some(200.0)),
            ("LedIndicador", None, None, None),
        ] {
            let _ = r.register(DeviceEntry::actor(
                name,
                ActorLimits { min, max, safety_limit: safety },
            ));
        }
        r
    }

    fn occupied(&self, name: &str) -> bool {
        self.devices.contains_key(name) || self.aliases.contains_key(name)
    }

    /// Registra dispositivo canônico; valida unicidade de nome e aliases.
    pub fn register(&mut self, entry: DeviceEntry) -> Result<(), RegistryError> {
        if self.occupied(&entry.name) {
            return Err(RegistryError::DuplicateName(entry.name));
        }
        for a in &entry.aliases {
            if self.occupied(a) {
                return Err(RegistryError::ConflictingAlias(a.clone()));
            }
        }
        for a in &entry.aliases {
            self.aliases.insert(a.clone(), entry.name.clone());
        }
        self.devices.insert(entry.name.clone(), entry);
        Ok(())
    }

    /// Define alias de dispositivo já registrado (`attention` →
    /// `human_attention`); leitura por alias é idêntica à do canônico.
    pub fn set_alias(&mut self, alias: &str, canonical: &str) -> Result<(), RegistryError> {
        // Alias de alias é encadeamento — rejeitado antes de "desconhecido".
        if self.aliases.contains_key(canonical) {
            return Err(RegistryError::ChainedAlias(canonical.into()));
        }
        if !self.devices.contains_key(canonical) {
            return Err(RegistryError::UnknownAlias {
                alias: alias.into(),
                canonical: canonical.into(),
            });
        }
        if self.occupied(alias) {
            return Err(RegistryError::ConflictingAlias(alias.into()));
        }
        self.aliases.insert(alias.into(), canonical.into());
        if let Some(d) = self.devices.get_mut(canonical) {
            d.aliases.push(alias.into());
        }
        Ok(())
    }

    /// Resolve alias → canônico (o próprio nome passa direto).
    pub fn canonical_of<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).map(String::as_str).unwrap_or(name)
    }

    /// Dispositivo por nome simbólico (canônico ou alias).
    pub fn get(&self, name: &str) -> Option<&DeviceEntry> {
        self.devices.get(self.canonical_of(name))
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut DeviceEntry> {
        let canonical = self.canonical_of(name).to_string();
        self.devices.get_mut(&canonical)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Todos os dispositivos canônicos (ordem alfabética estável).
    pub fn devices(&self) -> impl Iterator<Item = &DeviceEntry> {
        self.devices.values()
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// Projeção no formato plano do runtime (`vbl_runtime::fxp::Registry`),
    /// **incluindo aliases** — leitura por alias é idêntica à do canônico
    /// (FORMAL §6) e o loader valida referências pelos dois nomes.
    pub fn to_runtime_registry(&self) -> RuntimeRegistry {
        let mut r = RuntimeRegistry::default();
        for d in self.devices.values() {
            match &d.kind {
                DeviceKind::Sensor { quantity, unit, .. } => {
                    let info = SensorInfo {
                        quantity: quantity.clone(),
                        unit: unit.clone(),
                    };
                    r.sensores.insert(d.name.clone(), info.clone());
                    for a in &d.aliases {
                        r.sensores.insert(a.clone(), info.clone());
                    }
                }
                DeviceKind::Actor { limits } => {
                    r.actors.insert(d.name.clone(), limits.clone());
                    for a in &d.aliases {
                        r.actors.insert(a.clone(), limits.clone());
                    }
                }
            }
        }
        r
    }
}

/// Configuração do barramento/registro (arquivo de linhas `key = value`;
/// `#` comenta; sem dependências externas).
///
/// ```text
/// # vbl-fxp — exemplo
/// mode = hibrido
/// cache_ttl_ms = 100
/// cpu_temp.mode = real
/// cpu_temp.endpoint = auto            # ou thermal_zone:/sys/class/thermal/thermal_zone0
/// human_attention.alias_of = attention
/// VentoinhaReserva.mode = real
/// VentoinhaReserva.endpoint = unix:/tmp/fxpd.sock
/// VentoinhaReserva.max = 255
/// fallback.Ventoinha = VentoinhaReserva
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FxpConfig {
    pub mode: Option<OperationMode>,
    pub cache_ttl_ms: Option<u64>,
    pub read_timeout_ms: Option<u64>,
    pub act_timeout_local_ms: Option<u64>,
    pub act_timeout_remote_ms: Option<u64>,
    pub queue_timeout_ms: Option<u64>,
    pub retries: Option<u32>,
    /// Por dispositivo (nome canônico ou novo dispositivo).
    pub devices: BTreeMap<String, DeviceCfg>,
    /// `fallback.<actor> = alt1, alt2`.
    pub fallback: BTreeMap<String, Vec<String>>,
}

/// Bloco de configuração de um dispositivo.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceCfg {
    pub alias_of: Option<String>,
    pub mode: Option<DeviceMode>,
    pub endpoint: Option<Endpoint>,
    pub quantity: Option<String>,
    pub unit: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub safety_limit: Option<f64>,
    /// Precisão típica (sensor novo).
    pub precision_pct: Option<f64>,
}

impl FxpConfig {
    /// Faz o parse de linhas `key = value`. Chaves compostas:
    /// `<device>.<field>` e `fallback.<actor>`.
    pub fn parse(text: &str) -> Result<Self, RegistryError> {
        let mut cfg = Self::default();
        for (i, bruta) in text.lines().enumerate() {
            let line = bruta.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| RegistryError::InvalidConfig(format!("linha {}: sem '='", i + 1)))?;
            let key = key.trim();
            let value = value.trim();
            let num = |field: &str| -> Result<u64, RegistryError> {
                value.parse::<u64>().map_err(|_| {
                    RegistryError::InvalidConfig(format!(
                        "linha {}: {field} espera inteiro, got '{value}'",
                        i + 1
                    ))
                })
            };
            match key {
                "mode" => cfg.mode = Some(OperationMode::parse(value)?),
                "cache_ttl_ms" => cfg.cache_ttl_ms = Some(num("cache_ttl_ms")?),
                "read_timeout_ms" => cfg.read_timeout_ms = Some(num("read_timeout_ms")?),
                "act_timeout_local_ms" => cfg.act_timeout_local_ms = Some(num(key)?),
                "act_timeout_remote_ms" => cfg.act_timeout_remote_ms = Some(num(key)?),
                "queue_timeout_ms" => cfg.queue_timeout_ms = Some(num(key)?),
                "retries" => {
                    cfg.retries = Some(u32::try_from(num("retries")?).map_err(|_| {
                        RegistryError::InvalidConfig(format!("linha {}: retries muito grande", i + 1))
                    })?)
                }
                _ => {
                    if let Some(dev) = key.strip_prefix("fallback.") {
                        let alts: Vec<String> = value
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect();
                        if alts.is_empty() {
                            return Err(RegistryError::InvalidConfig(format!(
                                "linha {}: fallback sem alternativos",
                                i + 1
                            )));
                        }
                        cfg.fallback.insert(dev.into(), alts);
                    } else {
                        let (name, field) = key
                            .split_once('.')
                            .ok_or_else(|| {
                                RegistryError::InvalidConfig(format!(
                                    "linha {}: chave desconhecida '{key}'",
                                    i + 1
                                ))
                            })?;
                        let d = cfg.devices.entry(name.into()).or_default();
                        match field {
                            "alias_de" => d.alias_of = Some(value.into()),
                            "mode" => d.mode = Some(DeviceMode::parse(value)?),
                            "endpoint" => d.endpoint = Some(Endpoint::parse(value)?),
                            "grandeza" => d.quantity = Some(value.into()),
                            "unidade" => d.unit = Some(value.into()),
                            "precisao_pct" => {
                                d.precision_pct = Some(value.parse().map_err(|_| {
                                    RegistryError::InvalidConfig(format!(
                                        "linha {}: precisao_pct espera número",
                                        i + 1
                                    ))
                                })?)
                            }
                            "min" | "max" | "safety_limit" => {
                                let v: f64 = value.parse().map_err(|_| {
                                    RegistryError::InvalidConfig(format!(
                                        "linha {}: {field} espera número",
                                        i + 1
                                    ))
                                })?;
                                match field {
                                    "min" => d.min = Some(v),
                                    "max" => d.max = Some(v),
                                    _ => d.safety_limit = Some(v),
                                }
                            }
                            other => {
                                return Err(RegistryError::InvalidConfig(format!(
                                    "linha {}: campo de dispositivo desconhecido '{other}'",
                                    i + 1
                                )))
                            }
                        }
                    }
                }
            }
        }
        cfg.validate()?;
        Ok(cfg)
    }

    /// Regras estruturais: alias não encadeia; dispositivo novo precisa se
    /// declarar sensor (grandeza) ou ator (limites/endpoint de atuação).
    fn validate(&self) -> Result<(), RegistryError> {
        for (name, d) in &self.devices {
            if let Some(canonical) = &d.alias_of {
                if let Some(other) = self.devices.get(canonical) {
                    if other.alias_of.is_some() {
                        return Err(RegistryError::ChainedAlias(name.clone()));
                    }
                }
                if d.mode.is_some() || d.endpoint.is_some() {
                    return Err(RegistryError::InvalidConfig(format!(
                        "alias '{name}' não aceita mode/endpoint"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Aplica a config sobre um registro: aliases, overrides de dispositivos
    /// existentes e registro de extensões novas (PLAN §3.1: diretório
    /// dinâmico). Dispositivo novo deve se declarar sensor (grandeza) ou
    /// ator (algum limite) — senão é config inválida.
    pub fn apply(&self, registry: &mut DeviceRegistry) -> Result<(), RegistryError> {
        // 1) aliases primeiro (podem ser referenciados por novos dispositivos? não —
        //    mas fallback pode citar dispositivo novo; ordem: dispositivos → fallback)
        for (name, d) in &self.devices {
            if let Some(canonical) = &d.alias_of {
                registry.set_alias(name, canonical)?;
                continue;
            }
            if let Some(entry) = registry.get_mut(name) {
                // Override de dispositivo existente (obrigatório ou extensão).
                if let Some(mode) = d.mode {
                    entry.mode = mode;
                }
                if let Some(ep) = &d.endpoint {
                    entry.endpoint = ep.clone();
                }
                match &mut entry.kind {
                    DeviceKind::Sensor { quantity, unit, precision_pct, range } => {
                        if let Some(g) = &d.quantity {
                            *quantity = g.clone();
                        }
                        if let Some(u) = &d.unit {
                            *unit = u.clone();
                        }
                        if let Some(p) = d.precision_pct {
                            *precision_pct = p;
                        }
                        // Para sensores, min/max declaram a faixa física.
                        if d.min.is_some() || d.max.is_some() {
                            range.0 = d.min;
                            range.1 = d.max;
                        }
                        if d.safety_limit.is_some() {
                            return Err(RegistryError::InvalidConfig(format!(
                                "sensor '{name}' não aceita safety_limit (exclusivo de ator)"
                            )));
                        }
                    }
                    DeviceKind::Actor { limits } => {
                        if let Some(v) = d.min {
                            limits.min = Some(v);
                        }
                        if let Some(v) = d.max {
                            limits.max = Some(v);
                        }
                        if let Some(v) = d.safety_limit {
                            limits.safety_limit = Some(v);
                        }
                        if d.quantity.is_some() || d.unit.is_some() || d.precision_pct.is_some()
                        {
                            return Err(RegistryError::InvalidConfig(format!(
                                "ator '{name}' não aceita grandeza/unidade/precisao_pct"
                            )));
                        }
                    }
                }
            } else {
                // Extensão nova (ex.: `VentoinhaReserva`, `solar_panel`).
                let mode = d.mode.unwrap_or_default();
                let endpoint = d.endpoint.clone().unwrap_or_default();
                let kind = if let Some(g) = &d.quantity {
                    if d.safety_limit.is_some() {
                        return Err(RegistryError::InvalidConfig(format!(
                            "sensor '{name}' não aceita safety_limit (exclusivo de ator)"
                        )));
                    }
                    DeviceKind::Sensor {
                        quantity: g.clone(),
                        unit: d.unit.clone().unwrap_or_else(|| "unidade".into()),
                        range: (d.min, d.max),
                        precision_pct: d.precision_pct.unwrap_or(0.0),
                    }
                } else if d.min.is_some() || d.max.is_some() || d.safety_limit.is_some() {
                    DeviceKind::Actor {
                        limits: ActorLimits {
                            min: d.min,
                            max: d.max,
                            safety_limit: d.safety_limit,
                        },
                    }
                } else if endpoint.is_remote() {
                    // Ator remoto sem limites declarados (ex.: LedIndicador remoto).
                    DeviceKind::Actor { limits: ActorLimits::default() }
                } else {
                    return Err(RegistryError::InvalidConfig(format!(
                        "dispositivo novo '{name}' precisa de grandeza (sensor) ou limites (ator)"
                    )));
                };
                registry.register(DeviceEntry {
                    name: name.clone(),
                    kind,
                    mode,
                    endpoint,
                    fallback: Vec::new(),
                    aliases: Vec::new(),
                })?;
            }
        }
        // 2) política de fallback (FORMAL §4.3): validada contra o registro.
        for (actor, alts) in &self.fallback {
            if !registry.contains(actor) {
                return Err(RegistryError::InvalidConfig(format!(
                    "fallback de ator não registrado: '{actor}'"
                )));
            }
            for alt in alts {
                if !registry.contains(alt) {
                    return Err(RegistryError::UnknownFallback {
                        actor: actor.clone(),
                        alternativo: alt.clone(),
                    });
                }
            }
            let entry = registry.get_mut(actor).expect("checado acima");
            if !matches!(entry.kind, DeviceKind::Actor { .. }) {
                return Err(RegistryError::InvalidConfig(format!(
                    "fallback só se aplica a atores: '{actor}' é sensor"
                )));
            }
            entry.fallback = alts.clone();
        }
        Ok(())
    }
}
