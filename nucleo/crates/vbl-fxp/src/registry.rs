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
pub enum ModoOperacao {
    /// Todo dispositivo no endpoint físico; indisponível ⇒ falha honesta.
    Real,
    /// Todo dispositivo sintético, marcado no Caderno (CI-safe; default).
    #[default]
    Simulado,
    /// Por dispositivo: reais onde há endpoint, simulados no restante.
    Hibrido,
}

impl ModoOperacao {
    pub fn parse(s: &str) -> Result<Self, ErroRegistro> {
        match s {
            "real" => Ok(Self::Real),
            "simulado" => Ok(Self::Simulado),
            "hibrido" | "híbrido" => Ok(Self::Hibrido),
            other => Err(ErroRegistro::ConfigInvalida(format!(
                "modo desconhecido: '{other}' (use real | simulado | hibrido)"
            ))),
        }
    }

    pub fn nome(&self) -> &'static str {
        match self {
            ModoOperacao::Real => "real",
            ModoOperacao::Simulado => "simulado",
            ModoOperacao::Hibrido => "hibrido",
        }
    }
}

/// Modo do dispositivo individual (o global modula: em `Simulado` puro tudo
/// é sintético; em `Real` dispositivo sem endpoint real é inacessível; em
/// `Hibrido` vale o modo individual).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceMode {
    Real,
    #[default]
    Simulado,
}

impl DeviceMode {
    pub fn parse(s: &str) -> Result<Self, ErroRegistro> {
        match s {
            "real" => Ok(Self::Real),
            "simulado" => Ok(Self::Simulado),
            other => Err(ErroRegistro::ConfigInvalida(format!(
                "modo de dispositivo desconhecido: '{other}'"
            ))),
        }
    }

    pub fn nome(&self) -> &'static str {
        match self {
            DeviceMode::Real => "real",
            DeviceMode::Simulado => "simulado",
        }
    }
}

/// Grandeza/limites declarados do dispositivo (FORMAL §6).
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceKind {
    Sensor {
        grandeza: String,
        unidade: String,
        /// Faixa física esperada (documentativa; leitura fora dela é alerta,
        /// nunca fabricação — FORMAL §4.7).
        faixa: (Option<f64>, Option<f64>),
        /// Precisão típica em percentual (§6; 0.0 = dependente do backend).
        precisao_pct: f64,
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
    Simulado,
    /// Auto-descoberta no host (só para os dispositivos obrigatórios; falha
    /// de descoberta ⇒ dispositivo registrado porém inacessível — §4.7).
    Auto,
    /// thermal_zone: diretório com `temp` (mili°C) e `type`.
    ThermalZone { dir: PathBuf },
    /// RAPL consumo: diretório com `energy_uj` (+ `max_energy_range_uj` p/ wrap).
    RaplEnergy { dir: PathBuf },
    /// RAPL power cap: arquivo `constraint_0_power_limit_uw` (µW).
    RaplConstraint { file: PathBuf },
    /// hwmon PWM: arquivo `pwmN` (0–255).
    HwmonPwm { file: PathBuf },
    /// LED class: diretório com `brightness` e `max_brightness`.
    LedClass { dir: PathBuf },
    /// Peer remoto falando schema v1 (`unix:/caminho` ou `tcp:host:porta`).
    Remote { addr: RemoteAddr },
}

impl Endpoint {
    /// Formato textual do config: `simulado` | `auto` |
    /// `thermal_zone:<dir>` | `rapl_energy:<dir>` | `rapl_constraint:<arq>` |
    /// `hwmon_pwm:<arq>` | `led:<dir>` | `unix:<caminho>` | `tcp:<host>:<porta>`.
    pub fn parse(s: &str) -> Result<Self, ErroRegistro> {
        // Esquemas sem caminho.
        match s {
            "simulado" => return Ok(Endpoint::Simulado),
            "auto" => return Ok(Endpoint::Auto),
            _ => {}
        }
        let (esquema, resto) = s
            .split_once(':')
            .ok_or_else(|| ErroRegistro::EndpointInvalido(s.into()))?;
        let path = |v: &str| PathBuf::from(v);
        match esquema {
            "thermal_zone" => Ok(Endpoint::ThermalZone { dir: path(resto) }),
            "rapl_energy" => Ok(Endpoint::RaplEnergy { dir: path(resto) }),
            "rapl_constraint" => Ok(Endpoint::RaplConstraint { file: path(resto) }),
            "hwmon_pwm" => Ok(Endpoint::HwmonPwm { file: path(resto) }),
            "led" => Ok(Endpoint::LedClass { dir: path(resto) }),
            "unix" => Ok(Endpoint::Remote { addr: RemoteAddr::Unix(path(resto)) }),
            "tcp" => {
                let (host, port) = resto.rsplit_once(':').ok_or_else(|| {
                    ErroRegistro::EndpointInvalido(format!("{s} (esperado tcp:host:porta)"))
                })?;
                let port: u16 = port.parse().map_err(|_| {
                    ErroRegistro::EndpointInvalido(format!("{s} (porta inválida)"))
                })?;
                Ok(Endpoint::Remote { addr: RemoteAddr::Tcp { host: host.into(), port } })
            }
            _ => Err(ErroRegistro::EndpointInvalido(s.into())),
        }
    }

    /// Descrição canônica (re-parseável por [`Endpoint::parse`]; usada no
    /// Caderno, no `vbl fxp-probe` e na documentação).
    pub fn descricao(&self) -> String {
        match self {
            Endpoint::Simulado => "simulado".into(),
            Endpoint::Auto => "auto".into(),
            Endpoint::ThermalZone { dir } => format!("thermal_zone:{}", dir.display()),
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
    pub fn sensor(name: &str, grandeza: &str, unidade: &str, precisao_pct: f64) -> Self {
        Self {
            name: name.into(),
            kind: DeviceKind::Sensor {
                grandeza: grandeza.into(),
                unidade: unidade.into(),
                faixa: (None, None),
                precisao_pct,
            },
            mode: DeviceMode::Simulado,
            endpoint: Endpoint::Simulado,
            fallback: Vec::new(),
            aliases: Vec::new(),
        }
    }

    /// Ator simulado com limites do §6.
    pub fn ator(name: &str, limits: ActorLimits) -> Self {
        Self {
            name: name.into(),
            kind: DeviceKind::Actor { limits },
            mode: DeviceMode::Simulado,
            endpoint: Endpoint::Simulado,
            fallback: Vec::new(),
            aliases: Vec::new(),
        }
    }
}

/// Erro de registro/configuração.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroRegistro {
    /// Nome canônico já ocupado por outro dispositivo.
    NomeDuplicado(String),
    /// Alias/nome colide com nome canônico ou alias existente.
    AliasConflitante(String),
    /// Alias aponta para dispositivo inexistente.
    AliasDesconhecido { alias: String, canonical: String },
    /// Fallback cita alternativo fora do registro (FORMAL §4.3).
    FallbackDesconhecido { ator: String, alternativo: String },
    /// Alias definido sobre outro alias (encadeamento não é permitido).
    AliasEncadeado(String),
    /// String de endpoint fora do formato.
    EndpointInvalido(String),
    /// Config malformada (linha, valor, dispositivo subespecificado…).
    ConfigInvalida(String),
}

impl std::fmt::Display for ErroRegistro {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErroRegistro::NomeDuplicado(n) => {
                write!(f, "nome canônico '{n}' já registrado")
            }
            ErroRegistro::AliasConflitante(n) => {
                write!(f, "'{n}' colide com nome/alias já registrado")
            }
            ErroRegistro::AliasDesconhecido { alias, canonical } => {
                write!(f, "alias '{alias}' aponta para dispositivo inexistente '{canonical}'")
            }
            ErroRegistro::FallbackDesconhecido { ator, alternativo } => {
                write!(f, "fallback de '{ator}' cita '{alternativo}', fora do registro (FORMAL §4.3)")
            }
            ErroRegistro::AliasEncadeado(n) => {
                write!(f, "alias '{n}' não pode apontar para outro alias")
            }
            ErroRegistro::EndpointInvalido(e) => {
                write!(f, "endpoint inválido: '{e}' (simulado | auto | thermal_zone:… | rapl_energy:… | rapl_constraint:… | hwmon_pwm:… | led:… | unix:… | tcp:host:porta)")
            }
            ErroRegistro::ConfigInvalida(m) => write!(f, "config inválida: {m}"),
        }
    }
}

impl std::error::Error for ErroRegistro {}

/// Registro do FXP: mapeamento nome simbólico → dispositivo concreto.
#[derive(Debug, Clone, Default)]
pub struct DeviceRegistry {
    devices: BTreeMap<String, DeviceEntry>,
    /// alias → canônico (resolução O(1); FORMAL §6).
    aliases: BTreeMap<String, String>,
}

impl DeviceRegistry {
    pub fn novo() -> Self {
        Self::default()
    }

    /// Registro mínimo obrigatório (FORMAL §6) — modo simulado (CI-safe).
    /// Precisões típicas: cpu_temp ±2%, cpu_power ±5%, attention dependente
    /// do backend (0.0 = não declarado).
    pub fn minimo() -> Self {
        let mut r = Self::novo();
        for (name, grandeza, unidade, precisao) in [
            ("cpu_temp", "temperatura", "°C", 2.0),
            ("cpu_power", "potencia", "W", 5.0),
            ("attention", "atencao", "%", 0.0),
        ] {
            let _ = r.registrar(DeviceEntry::sensor(name, grandeza, unidade, precisao));
        }
        for (name, min, max, safety) in [
            ("CpuPowerCap", Some(10.0), Some(250.0), Some(200.0)),
            ("Ventoinha", Some(0.0), Some(255.0), Some(200.0)),
            ("LedIndicador", None, None, None),
        ] {
            let _ = r.registrar(DeviceEntry::ator(
                name,
                ActorLimits { min, max, safety_limit: safety },
            ));
        }
        r
    }

    fn ocupado(&self, nome: &str) -> bool {
        self.devices.contains_key(nome) || self.aliases.contains_key(nome)
    }

    /// Registra dispositivo canônico; valida unicidade de nome e aliases.
    pub fn registrar(&mut self, entry: DeviceEntry) -> Result<(), ErroRegistro> {
        if self.ocupado(&entry.name) {
            return Err(ErroRegistro::NomeDuplicado(entry.name));
        }
        for a in &entry.aliases {
            if self.ocupado(a) {
                return Err(ErroRegistro::AliasConflitante(a.clone()));
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
    pub fn definir_alias(&mut self, alias: &str, canonical: &str) -> Result<(), ErroRegistro> {
        // Alias de alias é encadeamento — rejeitado antes de "desconhecido".
        if self.aliases.contains_key(canonical) {
            return Err(ErroRegistro::AliasEncadeado(canonical.into()));
        }
        if !self.devices.contains_key(canonical) {
            return Err(ErroRegistro::AliasDesconhecido {
                alias: alias.into(),
                canonical: canonical.into(),
            });
        }
        if self.ocupado(alias) {
            return Err(ErroRegistro::AliasConflitante(alias.into()));
        }
        self.aliases.insert(alias.into(), canonical.into());
        if let Some(d) = self.devices.get_mut(canonical) {
            d.aliases.push(alias.into());
        }
        Ok(())
    }

    /// Resolve alias → canônico (o próprio nome passa direto).
    pub fn canonical_de<'a>(&'a self, nome: &'a str) -> &'a str {
        self.aliases.get(nome).map(String::as_str).unwrap_or(nome)
    }

    /// Dispositivo por nome simbólico (canônico ou alias).
    pub fn get(&self, nome: &str) -> Option<&DeviceEntry> {
        self.devices.get(self.canonical_de(nome))
    }

    pub fn get_mut(&mut self, nome: &str) -> Option<&mut DeviceEntry> {
        let canonical = self.canonical_de(nome).to_string();
        self.devices.get_mut(&canonical)
    }

    pub fn contains(&self, nome: &str) -> bool {
        self.get(nome).is_some()
    }

    /// Todos os dispositivos canônicos (ordem alfabética estável).
    pub fn dispositivos(&self) -> impl Iterator<Item = &DeviceEntry> {
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
                DeviceKind::Sensor { grandeza, unidade, .. } => {
                    let info = SensorInfo {
                        grandeza: grandeza.clone(),
                        unidade: unidade.clone(),
                    };
                    r.sensores.insert(d.name.clone(), info.clone());
                    for a in &d.aliases {
                        r.sensores.insert(a.clone(), info.clone());
                    }
                }
                DeviceKind::Actor { limits } => {
                    r.atores.insert(d.name.clone(), limits.clone());
                    for a in &d.aliases {
                        r.atores.insert(a.clone(), limits.clone());
                    }
                }
            }
        }
        r
    }
}

/// Configuração do barramento/registro (arquivo de linhas `chave = valor`;
/// `#` comenta; sem dependências externas).
///
/// ```text
/// # vbl-fxp — exemplo
/// mode = hibrido
/// cache_ttl_ms = 100
/// cpu_temp.mode = real
/// cpu_temp.endpoint = auto            # ou thermal_zone:/sys/class/thermal/thermal_zone0
/// human_attention.alias_de = attention
/// VentoinhaReserva.mode = real
/// VentoinhaReserva.endpoint = unix:/tmp/fxpd.sock
/// VentoinhaReserva.max = 255
/// fallback.Ventoinha = VentoinhaReserva
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FxpConfig {
    pub mode: Option<ModoOperacao>,
    pub cache_ttl_ms: Option<u64>,
    pub read_timeout_ms: Option<u64>,
    pub act_timeout_local_ms: Option<u64>,
    pub act_timeout_remote_ms: Option<u64>,
    pub queue_timeout_ms: Option<u64>,
    pub retries: Option<u32>,
    /// Por dispositivo (nome canônico ou novo dispositivo).
    pub devices: BTreeMap<String, DeviceCfg>,
    /// `fallback.<ator> = alt1, alt2`.
    pub fallback: BTreeMap<String, Vec<String>>,
}

/// Bloco de configuração de um dispositivo.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceCfg {
    pub alias_de: Option<String>,
    pub mode: Option<DeviceMode>,
    pub endpoint: Option<Endpoint>,
    pub grandeza: Option<String>,
    pub unidade: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub safety_limit: Option<f64>,
    /// Precisão típica (sensor novo).
    pub precisao_pct: Option<f64>,
}

impl FxpConfig {
    /// Faz o parse de linhas `chave = valor`. Chaves compostas:
    /// `<device>.<campo>` e `fallback.<ator>`.
    pub fn parse(texto: &str) -> Result<Self, ErroRegistro> {
        let mut cfg = Self::default();
        for (i, bruta) in texto.lines().enumerate() {
            let linha = bruta.split('#').next().unwrap_or("").trim();
            if linha.is_empty() {
                continue;
            }
            let (chave, valor) = linha
                .split_once('=')
                .ok_or_else(|| ErroRegistro::ConfigInvalida(format!("linha {}: sem '='", i + 1)))?;
            let chave = chave.trim();
            let valor = valor.trim();
            let num = |campo: &str| -> Result<u64, ErroRegistro> {
                valor.parse::<u64>().map_err(|_| {
                    ErroRegistro::ConfigInvalida(format!(
                        "linha {}: {campo} espera inteiro, got '{valor}'",
                        i + 1
                    ))
                })
            };
            match chave {
                "mode" => cfg.mode = Some(ModoOperacao::parse(valor)?),
                "cache_ttl_ms" => cfg.cache_ttl_ms = Some(num("cache_ttl_ms")?),
                "read_timeout_ms" => cfg.read_timeout_ms = Some(num("read_timeout_ms")?),
                "act_timeout_local_ms" => cfg.act_timeout_local_ms = Some(num(chave)?),
                "act_timeout_remote_ms" => cfg.act_timeout_remote_ms = Some(num(chave)?),
                "queue_timeout_ms" => cfg.queue_timeout_ms = Some(num(chave)?),
                "retries" => {
                    cfg.retries = Some(u32::try_from(num("retries")?).map_err(|_| {
                        ErroRegistro::ConfigInvalida(format!("linha {}: retries muito grande", i + 1))
                    })?)
                }
                _ => {
                    if let Some(dev) = chave.strip_prefix("fallback.") {
                        let alts: Vec<String> = valor
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect();
                        if alts.is_empty() {
                            return Err(ErroRegistro::ConfigInvalida(format!(
                                "linha {}: fallback sem alternativos",
                                i + 1
                            )));
                        }
                        cfg.fallback.insert(dev.into(), alts);
                    } else {
                        let (nome, campo) = chave
                            .split_once('.')
                            .ok_or_else(|| {
                                ErroRegistro::ConfigInvalida(format!(
                                    "linha {}: chave desconhecida '{chave}'",
                                    i + 1
                                ))
                            })?;
                        let d = cfg.devices.entry(nome.into()).or_default();
                        match campo {
                            "alias_de" => d.alias_de = Some(valor.into()),
                            "mode" => d.mode = Some(DeviceMode::parse(valor)?),
                            "endpoint" => d.endpoint = Some(Endpoint::parse(valor)?),
                            "grandeza" => d.grandeza = Some(valor.into()),
                            "unidade" => d.unidade = Some(valor.into()),
                            "precisao_pct" => {
                                d.precisao_pct = Some(valor.parse().map_err(|_| {
                                    ErroRegistro::ConfigInvalida(format!(
                                        "linha {}: precisao_pct espera número",
                                        i + 1
                                    ))
                                })?)
                            }
                            "min" | "max" | "safety_limit" => {
                                let v: f64 = valor.parse().map_err(|_| {
                                    ErroRegistro::ConfigInvalida(format!(
                                        "linha {}: {campo} espera número",
                                        i + 1
                                    ))
                                })?;
                                match campo {
                                    "min" => d.min = Some(v),
                                    "max" => d.max = Some(v),
                                    _ => d.safety_limit = Some(v),
                                }
                            }
                            other => {
                                return Err(ErroRegistro::ConfigInvalida(format!(
                                    "linha {}: campo de dispositivo desconhecido '{other}'",
                                    i + 1
                                )))
                            }
                        }
                    }
                }
            }
        }
        cfg.validar()?;
        Ok(cfg)
    }

    /// Regras estruturais: alias não encadeia; dispositivo novo precisa se
    /// declarar sensor (grandeza) ou ator (limites/endpoint de atuação).
    fn validar(&self) -> Result<(), ErroRegistro> {
        for (nome, d) in &self.devices {
            if let Some(canonical) = &d.alias_de {
                if let Some(outro) = self.devices.get(canonical) {
                    if outro.alias_de.is_some() {
                        return Err(ErroRegistro::AliasEncadeado(nome.clone()));
                    }
                }
                if d.mode.is_some() || d.endpoint.is_some() {
                    return Err(ErroRegistro::ConfigInvalida(format!(
                        "alias '{nome}' não aceita mode/endpoint"
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
    pub fn aplicar(&self, registry: &mut DeviceRegistry) -> Result<(), ErroRegistro> {
        // 1) aliases primeiro (podem ser referenciados por novos dispositivos? não —
        //    mas fallback pode citar dispositivo novo; ordem: dispositivos → fallback)
        for (nome, d) in &self.devices {
            if let Some(canonical) = &d.alias_de {
                registry.definir_alias(nome, canonical)?;
                continue;
            }
            if let Some(entry) = registry.get_mut(nome) {
                // Override de dispositivo existente (obrigatório ou extensão).
                if let Some(mode) = d.mode {
                    entry.mode = mode;
                }
                if let Some(ep) = &d.endpoint {
                    entry.endpoint = ep.clone();
                }
                match &mut entry.kind {
                    DeviceKind::Sensor { grandeza, unidade, precisao_pct, faixa } => {
                        if let Some(g) = &d.grandeza {
                            *grandeza = g.clone();
                        }
                        if let Some(u) = &d.unidade {
                            *unidade = u.clone();
                        }
                        if let Some(p) = d.precisao_pct {
                            *precisao_pct = p;
                        }
                        // Para sensores, min/max declaram a faixa física.
                        if d.min.is_some() || d.max.is_some() {
                            faixa.0 = d.min;
                            faixa.1 = d.max;
                        }
                        if d.safety_limit.is_some() {
                            return Err(ErroRegistro::ConfigInvalida(format!(
                                "sensor '{nome}' não aceita safety_limit (exclusivo de ator)"
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
                        if d.grandeza.is_some() || d.unidade.is_some() || d.precisao_pct.is_some()
                        {
                            return Err(ErroRegistro::ConfigInvalida(format!(
                                "ator '{nome}' não aceita grandeza/unidade/precisao_pct"
                            )));
                        }
                    }
                }
            } else {
                // Extensão nova (ex.: `VentoinhaReserva`, `solar_panel`).
                let mode = d.mode.unwrap_or_default();
                let endpoint = d.endpoint.clone().unwrap_or_default();
                let kind = if let Some(g) = &d.grandeza {
                    if d.safety_limit.is_some() {
                        return Err(ErroRegistro::ConfigInvalida(format!(
                            "sensor '{nome}' não aceita safety_limit (exclusivo de ator)"
                        )));
                    }
                    DeviceKind::Sensor {
                        grandeza: g.clone(),
                        unidade: d.unidade.clone().unwrap_or_else(|| "unidade".into()),
                        faixa: (d.min, d.max),
                        precisao_pct: d.precisao_pct.unwrap_or(0.0),
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
                    return Err(ErroRegistro::ConfigInvalida(format!(
                        "dispositivo novo '{nome}' precisa de grandeza (sensor) ou limites (ator)"
                    )));
                };
                registry.registrar(DeviceEntry {
                    name: nome.clone(),
                    kind,
                    mode,
                    endpoint,
                    fallback: Vec::new(),
                    aliases: Vec::new(),
                })?;
            }
        }
        // 2) política de fallback (FORMAL §4.3): validada contra o registro.
        for (ator, alts) in &self.fallback {
            if !registry.contains(ator) {
                return Err(ErroRegistro::ConfigInvalida(format!(
                    "fallback de ator não registrado: '{ator}'"
                )));
            }
            for alt in alts {
                if !registry.contains(alt) {
                    return Err(ErroRegistro::FallbackDesconhecido {
                        ator: ator.clone(),
                        alternativo: alt.clone(),
                    });
                }
            }
            let entry = registry.get_mut(ator).expect("checado acima");
            if !matches!(entry.kind, DeviceKind::Actor { .. }) {
                return Err(ErroRegistro::ConfigInvalida(format!(
                    "fallback só se aplica a atores: '{ator}' é sensor"
                )));
            }
            entry.fallback = alts.clone();
        }
        Ok(())
    }
}
