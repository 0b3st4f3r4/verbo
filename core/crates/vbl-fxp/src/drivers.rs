//! Drivers do FXP — backends reais (sysfs/thermal_zone, RAPL, hwmon PWM,
//! LED class) e o `AttentionSource` (PLAN §3.2/§3.3).
//!
//! Regras canônicas:
//! - **Conversão de unidade no driver**: `temp` em mili°C → °C; `energy_uj`
//!   em µJ → W por diferença finita; comando em W → µW. O registro declara a
//!   unidade canônica (FORMAL §6) e o parser/runtime só veem esse valor.
//! - **Nada é fabricado**: arquivo ausente, ilegível ou amostra insuficiente
//!   ⇒ [`SensorFailure::Inaccessible`] / [`ActorError`] — nunca `0.0` (FORMAL §4.7).
//! - **Fallback atua na rota de I/O** (endpoint alternativo), nunca falsifica
//!   leitura (AGENTS §1.2 EIF).
//! - Endpoints de teste: qualquer caminho pode apontar para uma árvore sysfs
//!   sintética em tmpdir — o mesmo código de leitura/escrita roda em CI
//!   (integração honesta sem hardware).

use std::path::{Path, PathBuf};

use vbl_runtime::fxp::{SensorFailure, Value};

/// Escrita de atuação **sem `O_CREAT`**: endpoint que sumiu (driver
/// desvinculado, sysfs recompilado) ⇒ [`ActorError::WriteFailed`] registrado
/// — nunca a recriação silenciosa de um arquivo regular que `fs::write`
/// produziria (honestidade de I/O, FORMAL §4.7).
fn write_endpoint(path: &Path, content: &str) -> Result<(), ActorError> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| ActorError::WriteFailed(format!("{}: {e}", path.display())))?;
    f.write_all(content.as_bytes())
        .map_err(|e| ActorError::WriteFailed(format!("{}: {e}", path.display())))
}

/// Leitura de sensor convertida para a unidade canônica do registro.
pub trait SensorDriver {
    /// Falha → [`SensorFailure`] (nunca leitura 0.0 — §4.7).
    fn read(&mut self) -> Result<f64, SensorFailure>;

    /// Descrição do endpoint (Caderno, `vbl fxp-probe`).
    fn description(&self) -> String;
}

/// Erro de atuação no endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorError {
    /// Escrita/heartbeat falhou no endpoint.
    WriteFailed(String),
    /// Valor fora do domínio do ator (ex.: cor inexistente; texto em ator
    /// numérico) — vira `ACT_ACK.InvalidValue`, nunca entrega silenciosa.
    InvalidValue(String),
}

impl std::fmt::Display for ActorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorError::WriteFailed(m) => write!(f, "escrita falhou: {m}"),
            ActorError::InvalidValue(m) => write!(f, "valor inválido: {m}"),
        }
    }
}

/// Atuação no endpoint + heartbeat (BDD Caso 3).
pub trait ActorDriver {
    fn apply(&mut self, value: &Value) -> Result<(), ActorError>;
    /// O ator responde? (endpoints de arquivo: caminho existe e é gravável)
    fn heartbeat(&mut self) -> bool;
    fn description(&self) -> String;
}

// ---------------------------------------------------------------------------
// Sensores reais
// ---------------------------------------------------------------------------

/// `cpu_temp` — thermal_zone (`/sys/class/thermal/thermal_zone*/temp`, m°C).
#[derive(Debug, Clone)]
pub struct ThermalZoneSensor {
    dir: PathBuf,
}

impl ThermalZoneSensor {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl SensorDriver for ThermalZoneSensor {
    fn read(&mut self) -> Result<f64, SensorFailure> {
        let bruto = std::fs::read_to_string(self.dir.join("temp"))
            .map_err(|_| SensorFailure::Inaccessible)?;
        let mili: f64 = bruto.trim().parse().map_err(|_| SensorFailure::Inaccessible)?;
        Ok(mili / 1000.0)
    }

    fn description(&self) -> String {
        format!("thermal_zone:{}", self.dir.display())
    }
}

/// `cpu_temp` (e afins) via hwmon — arquivo `tempN_input` (mili°C), ex.:
/// `/sys/class/hwmon/hwmon4/temp1_input` (k10temp, temperatura real da CPU).
#[derive(Debug, Clone)]
pub struct HwmonTempSensor {
    file: PathBuf,
}

impl HwmonTempSensor {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }
}

impl SensorDriver for HwmonTempSensor {
    fn read(&mut self) -> Result<f64, SensorFailure> {
        let bruto =
            std::fs::read_to_string(&self.file).map_err(|_| SensorFailure::Inaccessible)?;
        let mili: f64 = bruto.trim().parse().map_err(|_| SensorFailure::Inaccessible)?;
        Ok(mili / 1000.0)
    }

    fn description(&self) -> String {
        format!("hwmon_temp:{}", self.file.display())
    }
}

/// Fonte de atenção humana — interface abstrata (PLAN §3.2, `AttentionSource`).
/// O backend **simulado é obrigatório** como fallback em CI; EEG/eye tracking
/// são extensões opcionais que plugam nesta mesma trait.
pub trait AttentionSource {
    fn read(&mut self) -> Result<f64, SensorFailure>;
}

/// Backend simulado padrão: valor roteirizado pelo bus/simulador (0–100%).
#[derive(Debug, Clone, Default)]
pub struct SimulatedAttention {
    pub value_pct: f64,
}

impl AttentionSource for SimulatedAttention {
    fn read(&mut self) -> Result<f64, SensorFailure> {
        Ok(self.value_pct.clamp(0.0, 100.0))
    }
}

/// Relógio injetável (segundos arbitrários) — testes determinísticos do RAPL.
pub type Clock = Box<dyn Fn() -> f64 + Send>;

pub fn wall_clock() -> Clock {
    // Base capturada UMA vez: `Instant::now().elapsed()` sobre um instante
    // recém-criado mediria ~0 ns em toda chamada (bug latente que produzia
    // W absurdos — ΔE/Δt com Δt nanosegundos). elapsed() é monotônico.
    let start = std::time::Instant::now();
    Box::new(move || start.elapsed().as_secs_f64())
}

/// `cpu_power` — RAPL (`energy_uj` em µJ) via diferença finita entre amostras:
/// `W = ΔE[µJ] / 1e6 / Δt[s]`. A **primeira amostra apenas inicializa** —
/// sem ΔE não há leitura honesta, e o bus registra condição não avaliada
/// (§4.7). Wrap do contador tratado com `max_energy_range_uj`.
pub struct RaplEnergySensor {
    dir: PathBuf,
    now: Clock,
    previous: Option<(f64, u64)>,
}

impl RaplEnergySensor {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), now: wall_clock(), previous: None }
    }

    pub fn with_clock(dir: impl Into<PathBuf>, now: Clock) -> Self {
        Self { dir: dir.into(), now, previous: None }
    }

    fn read_uj(&self, arquivo: &str) -> Result<u64, SensorFailure> {
        let bruto = std::fs::read_to_string(self.dir.join(arquivo))
            .map_err(|_| SensorFailure::Inaccessible)?;
        bruto.trim().parse::<u64>().map_err(|_| SensorFailure::Inaccessible)
    }
}

/// Janela mínima entre amostras (s): o contador do RAPL avança em quanta
/// (~ms) — um par com Δt menor não mede potência. Re-leituras do mesmo tick
/// (auditoria × avaliação) são **degeneradas**: sem informação, não devem
/// sobrescrever a última média válida nem fabricar W absurdos (§4.7).
const MIN_DT_S: f64 = 1e-3;

impl SensorDriver for RaplEnergySensor {
    fn read(&mut self) -> Result<f64, SensorFailure> {
        let energy = self.read_uj("energy_uj")?;
        let t = (self.now)();
        let Some((t0, e0)) = self.previous else {
            self.previous = Some((t, energy));
            return Err(SensorFailure::Inaccessible); // amostra de aquecimento
        };
        let dt = t - t0;
        if dt < MIN_DT_S {
            // Par degenerado: mantém `previous` — a próxima amostra válida
            // cobre a janela inteira (sem update, sem invenção de potência).
            return Err(SensorFailure::Inaccessible);
        }
        let delta_e = if energy >= e0 {
            energy - e0
        } else {
            // Wrap: e1 + range − e0 (range do contador, não inventado).
            let range = self.read_uj("max_energy_range_uj")?;
            if range == 0 {
                return Err(SensorFailure::Inaccessible);
            }
            range - e0 + energy
        };
        self.previous = Some((t, energy));
        Ok(delta_e as f64 / 1e6 / dt)
    }

    fn description(&self) -> String {
        format!("rapl_energy:{}", self.dir.display())
    }
}

// ---------------------------------------------------------------------------
// Atores reais
// ---------------------------------------------------------------------------

/// `CpuPowerCap` — RAPL power capping (`constraint_0_power_limit_uw`, µW).
/// O comando chega em W (unidade do registro §6) e o driver converte.
pub struct RaplPowerCapActor {
    file: PathBuf,
}

impl RaplPowerCapActor {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }
}

impl ActorDriver for RaplPowerCapActor {
    fn apply(&mut self, value: &Value) -> Result<(), ActorError> {
        let Some(w) = value.as_num() else {
            return Err(ActorError::InvalidValue(format!(
                "CpuPowerCap espera valor numérico em W, recebeu {value}"
            )));
        };
        if !(0.0..).contains(&w) {
            return Err(ActorError::InvalidValue(format!("potência negativa: {w} W")));
        }
        let uw = format!("{}", (w * 1e6) as u64);
        write_endpoint(&self.file, &uw)
    }

    fn heartbeat(&mut self) -> bool {
        self.file.exists()
    }

    fn description(&self) -> String {
        format!("rapl_constraint:{}", self.file.display())
    }
}

/// `Fan` — PWM via hwmon (`/sys/class/hwmon/hwmon*/pwmN`, 0–255).
pub struct HwmonPwmActor {
    file: PathBuf,
}

impl HwmonPwmActor {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }
}

impl ActorDriver for HwmonPwmActor {
    fn apply(&mut self, value: &Value) -> Result<(), ActorError> {
        let Some(v) = value.as_num() else {
            return Err(ActorError::InvalidValue(format!(
                "Fan espera PWM numérico 0–255, recebeu {value}"
            )));
        };
        if !(0.0..=255.0).contains(&v) || v.fract() != 0.0 {
            return Err(ActorError::InvalidValue(format!(
                "PWM fora do domínio inteiro 0–255: {v}"
            )));
        }
        write_endpoint(&self.file, &format!("{}", v as u8))
    }

    fn heartbeat(&mut self) -> bool {
        self.file.exists()
    }

    fn description(&self) -> String {
        format!("hwmon_pwm:{}", self.file.display())
    }
}

/// `StatusLed` — LED class (`/sys/class/leds/*/brightness`).
/// Estado textual do registro (§6): cores nomeadas → brilho via mapa de
/// configuração (`verde`/`vermelho`/`amarelo`/`azul`/`apagado` por padrão);
/// número direto é aceito como brilho (extensão honesta e auditável).
pub struct LedClassActor {
    dir: PathBuf,
    mapa: std::collections::BTreeMap<String, u8>,
    max: u8,
}

impl LedClassActor {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let max = std::fs::read_to_string(dir.join("max_brightness"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .and_then(|m| u8::try_from(m).ok())
            .unwrap_or(255);
        let mut mapa = std::collections::BTreeMap::new();
        mapa.insert("off".to_string(), 0);
        mapa.insert("red".to_string(), max);
        mapa.insert("green".to_string(), (max / 4).max(1));
        mapa.insert("amarelo".to_string(), (max / 2).max(1));
        mapa.insert("azul".to_string(), (max / 3).max(1));
        Self { dir, mapa, max }
    }

    pub fn with_map(dir: impl Into<PathBuf>, mapa: std::collections::BTreeMap<String, u8>) -> Self {
        let mut d = Self::new(dir);
        d.mapa = mapa;
        d
    }

    pub fn max_brightness(&self) -> u8 {
        self.max
    }
}

impl ActorDriver for LedClassActor {
    fn apply(&mut self, value: &Value) -> Result<(), ActorError> {
        let brightness = match value {
            Value::Str(s) | Value::Ident(s) => {
                self.mapa.get(s.as_str()).copied().ok_or_else(|| {
                    ActorError::InvalidValue(format!(
                        "cor '{s}' fora do mapa do StatusLed ({:?})",
                        self.mapa.keys().collect::<Vec<_>>()
                    ))
                })?
            }
            Value::Num(n) if (0.0..=255.0).contains(n) && n.fract() == 0.0 => *n as u8,
            Value::Num(n) => {
                return Err(ActorError::InvalidValue(format!(
                    "brilho fora do domínio inteiro 0–255: {n}"
                )))
            }
        };
        if brightness > self.max {
            return Err(ActorError::InvalidValue(format!(
                "brilho {brightness} excede max_brightness = {}",
                self.max
            )));
        }
        write_endpoint(&self.dir.join("brightness"), &format!("{brightness}"))
    }

    fn heartbeat(&mut self) -> bool {
        self.dir.join("brightness").exists()
    }

    fn description(&self) -> String {
        format!("led:{}", self.dir.display())
    }
}

// ---------------------------------------------------------------------------
// Auto-descoberta (PLAN §3.2: endpoints concretos por nome obrigatório)
// ---------------------------------------------------------------------------

/// Descobre o endpoint real de um dispositivo obrigatório no host. Retorna
/// `None` quando não há hardware correspondente — o dispositivo segue
/// **registrado porém inacessível** (FORMAL §4.7), jamais simulado em modo
/// real. `attention` não é descobrível: o backend simulado é o padrão.
pub fn discover(name: &str) -> Option<crate::registry::Endpoint> {
    use crate::registry::Endpoint;
    match name {
        "cpu_temp" => discover_thermal_zone().map(|dir| Endpoint::ThermalZone { dir }),
        "cpu_power" => discover_rapl_energy().map(|dir| Endpoint::RaplEnergy { dir }),
        "CpuPowerCap" => {
            let f = Path::new("/sys/class/powercap/intel-rapl:0/constraint_0_power_limit_uw");
            f.exists().then(|| Endpoint::RaplConstraint { file: f.into() })
        }
        "Fan" => discover_pwm().map(|file| Endpoint::HwmonPwm { file }),
        "StatusLed" => discover_led().map(|dir| Endpoint::LedClass { dir }),
        _ => None,
    }
}

/// Primeira thermal_zone plausível: preferência `x86_pkg_temp`/`cpu_*`;
/// caso nenhum `type` case, a primeira com `temp` legível.
pub fn discover_thermal_zone() -> Option<PathBuf> {
    let mut fallback = None;
    for zone in listar_dirs("/sys/class/thermal", "thermal_zone") {
        if !zone.join("temp").exists() {
            continue;
        }
        let kind = std::fs::read_to_string(zone.join("type")).unwrap_or_default();
        let kind = kind.to_lowercase();
        if kind.contains("x86_pkg_temp") || kind.contains("cpu") || kind.contains("acpitz") {
            return Some(zone);
        }
        fallback.get_or_insert(zone);
    }
    fallback
}

/// Primeiro domínio RAPL com `energy_uj`.
pub fn discover_rapl_energy() -> Option<PathBuf> {
    listar_dirs("/sys/class/powercap", "intel-rapl")
        .into_iter()
        .find(|d| d.join("energy_uj").exists())
}

/// Primeiro hwmon com PWM exportado.
pub fn discover_pwm() -> Option<PathBuf> {
    for hwmon in listar_dirs("/sys/class/hwmon", "hwmon") {
        for n in 1..=4 {
            let pwm = hwmon.join(format!("pwm{n}"));
            if pwm.exists() {
                return Some(pwm);
            }
        }
    }
    None
}

/// Primeira LED class com brightness gravável.
pub fn discover_led() -> Option<PathBuf> {
    listar_dirs("/sys/class/leds", "").into_iter().find(|d| d.join("brightness").exists())
}

fn listar_dirs(base: &str, prefix: &str) -> Vec<PathBuf> {
    let Ok(entry) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = entry
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            // Nome não-UTF8 só casa quando não há prefixo a respeitar (LEDs).
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(prefix))
                    .unwrap_or(prefix.is_empty())
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// Fábrica: endpoint → driver
// ---------------------------------------------------------------------------

/// Driver de sensor para endpoint real (ou `None` para endpoint sem driver
/// de leitura — `Simulated` é do bus, `Remote` é do transporte).
pub fn sensor_from(
    endpoint: &crate::registry::Endpoint,
) -> Option<Box<dyn SensorDriver + Send>> {
    use crate::registry::Endpoint;
    match endpoint {
        Endpoint::ThermalZone { dir } => Some(Box::new(ThermalZoneSensor::new(dir.clone()))),
        Endpoint::HwmonTemp { file } => Some(Box::new(HwmonTempSensor::new(file.clone()))),
        Endpoint::RaplEnergy { dir } => Some(Box::new(RaplEnergySensor::new(dir.clone()))),
        _ => None,
    }
}

/// Driver de ator para endpoint real.
pub fn actor_from(endpoint: &crate::registry::Endpoint) -> Option<Box<dyn ActorDriver + Send>> {
    use crate::registry::Endpoint;
    match endpoint {
        Endpoint::RaplConstraint { file } => Some(Box::new(RaplPowerCapActor::new(file.clone()))),
        Endpoint::HwmonPwm { file } => Some(Box::new(HwmonPwmActor::new(file.clone()))),
        Endpoint::LedClass { dir } => Some(Box::new(LedClassActor::new(dir.clone()))),
        _ => None,
    }
}
