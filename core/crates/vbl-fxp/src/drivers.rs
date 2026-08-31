//! Drivers do FXP — backends reais (sysfs/thermal_zone, RAPL, hwmon PWM,
//! LED class) e o `AttentionSource` (PLAN §3.2/§3.3).
//!
//! Regras canônicas:
//! - **Conversão de unidade no driver**: `temp` em mili°C → °C; `energy_uj`
//!   em µJ → W por diferença finita; comando em W → µW. O registro declara a
//!   unidade canônica (FORMAL §6) e o parser/runtime só veem esse valor.
//! - **Nada é fabricado**: arquivo ausente, ilegível ou amostra insuficiente
//!   ⇒ [`FalhaSensor::Inacessivel`] / [`ErroAtor`] — nunca `0.0` (FORMAL §4.7).
//! - **Fallback atua na rota de I/O** (endpoint alternativo), nunca falsifica
//!   leitura (AGENTS §1.2 EIF).
//! - Endpoints de teste: qualquer caminho pode apontar para uma árvore sysfs
//!   sintética em tmpdir — o mesmo código de leitura/escrita roda em CI
//!   (integração honesta sem hardware).

use std::path::{Path, PathBuf};

use vbl_runtime::fxp::{FalhaSensor, Value};

/// Escrita de atuação **sem `O_CREAT`**: endpoint que sumiu (driver
/// desvinculado, sysfs recompilado) ⇒ [`ErroAtor::EscritaFalhou`] registrado
/// — nunca a recriação silenciosa de um arquivo regular que `fs::write`
/// produziria (honestidade de I/O, FORMAL §4.7).
fn escrever_endpoint(path: &Path, conteudo: &str) -> Result<(), ErroAtor> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| ErroAtor::EscritaFalhou(format!("{}: {e}", path.display())))?;
    f.write_all(conteudo.as_bytes())
        .map_err(|e| ErroAtor::EscritaFalhou(format!("{}: {e}", path.display())))
}

/// Leitura de sensor convertida para a unidade canônica do registro.
pub trait SensorDriver {
    /// Falha → [`FalhaSensor`] (nunca leitura 0.0 — §4.7).
    fn read(&mut self) -> Result<f64, FalhaSensor>;

    /// Descrição do endpoint (Caderno, `vbl fxp-probe`).
    fn descricao(&self) -> String;
}

/// Erro de atuação no endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroAtor {
    /// Escrita/heartbeat falhou no endpoint.
    EscritaFalhou(String),
    /// Valor fora do domínio do ator (ex.: cor inexistente; texto em ator
    /// numérico) — vira `ACT_ACK.ValorInvalido`, nunca entrega silenciosa.
    ValorInvalido(String),
}

impl std::fmt::Display for ErroAtor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErroAtor::EscritaFalhou(m) => write!(f, "escrita falhou: {m}"),
            ErroAtor::ValorInvalido(m) => write!(f, "valor inválido: {m}"),
        }
    }
}

/// Atuação no endpoint + heartbeat (BDD Caso 3).
pub trait ActorDriver {
    fn apply(&mut self, valor: &Value) -> Result<(), ErroAtor>;
    /// O ator responde? (endpoints de arquivo: caminho existe e é gravável)
    fn heartbeat(&mut self) -> bool;
    fn descricao(&self) -> String;
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
    pub fn novo(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl SensorDriver for ThermalZoneSensor {
    fn read(&mut self) -> Result<f64, FalhaSensor> {
        let bruto = std::fs::read_to_string(self.dir.join("temp"))
            .map_err(|_| FalhaSensor::Inacessivel)?;
        let mili: f64 = bruto.trim().parse().map_err(|_| FalhaSensor::Inacessivel)?;
        Ok(mili / 1000.0)
    }

    fn descricao(&self) -> String {
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
    pub fn novo(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }
}

impl SensorDriver for HwmonTempSensor {
    fn read(&mut self) -> Result<f64, FalhaSensor> {
        let bruto =
            std::fs::read_to_string(&self.file).map_err(|_| FalhaSensor::Inacessivel)?;
        let mili: f64 = bruto.trim().parse().map_err(|_| FalhaSensor::Inacessivel)?;
        Ok(mili / 1000.0)
    }

    fn descricao(&self) -> String {
        format!("hwmon_temp:{}", self.file.display())
    }
}

/// Fonte de atenção humana — interface abstrata (PLAN §3.2, `AttentionSource`).
/// O backend **simulado é obrigatório** como fallback em CI; EEG/eye tracking
/// são extensões opcionais que plugam nesta mesma trait.
pub trait AttentionSource {
    fn read(&mut self) -> Result<f64, FalhaSensor>;
}

/// Backend simulado padrão: valor roteirizado pelo bus/simulador (0–100%).
#[derive(Debug, Clone, Default)]
pub struct SimulatedAttention {
    pub valor_pct: f64,
}

impl AttentionSource for SimulatedAttention {
    fn read(&mut self) -> Result<f64, FalhaSensor> {
        Ok(self.valor_pct.clamp(0.0, 100.0))
    }
}

/// Relógio injetável (segundos arbitrários) — testes determinísticos do RAPL.
pub type Clock = Box<dyn Fn() -> f64 + Send>;

pub fn relogio_parede() -> Clock {
    // Base capturada UMA vez: `Instant::now().elapsed()` sobre um instante
    // recém-criado mediria ~0 ns em toda chamada (bug latente que produzia
    // W absurdos — ΔE/Δt com Δt nanosegundos). elapsed() é monotônico.
    let inicio = std::time::Instant::now();
    Box::new(move || inicio.elapsed().as_secs_f64())
}

/// `cpu_power` — RAPL (`energy_uj` em µJ) via diferença finita entre amostras:
/// `W = ΔE[µJ] / 1e6 / Δt[s]`. A **primeira amostra apenas inicializa** —
/// sem ΔE não há leitura honesta, e o bus registra condição não avaliada
/// (§4.7). Wrap do contador tratado com `max_energy_range_uj`.
pub struct RaplEnergySensor {
    dir: PathBuf,
    agora: Clock,
    anterior: Option<(f64, u64)>,
}

impl RaplEnergySensor {
    pub fn novo(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), agora: relogio_parede(), anterior: None }
    }

    pub fn com_relogio(dir: impl Into<PathBuf>, agora: Clock) -> Self {
        Self { dir: dir.into(), agora, anterior: None }
    }

    fn ler_uj(&self, arquivo: &str) -> Result<u64, FalhaSensor> {
        let bruto = std::fs::read_to_string(self.dir.join(arquivo))
            .map_err(|_| FalhaSensor::Inacessivel)?;
        bruto.trim().parse::<u64>().map_err(|_| FalhaSensor::Inacessivel)
    }
}

/// Janela mínima entre amostras (s): o contador do RAPL avança em quanta
/// (~ms) — um par com Δt menor não mede potência. Re-leituras do mesmo tick
/// (auditoria × avaliação) são **degeneradas**: sem informação, não devem
/// sobrescrever a última média válida nem fabricar W absurdos (§4.7).
const MIN_DT_S: f64 = 1e-3;

impl SensorDriver for RaplEnergySensor {
    fn read(&mut self) -> Result<f64, FalhaSensor> {
        let energia = self.ler_uj("energy_uj")?;
        let t = (self.agora)();
        let Some((t0, e0)) = self.anterior else {
            self.anterior = Some((t, energia));
            return Err(FalhaSensor::Inacessivel); // amostra de aquecimento
        };
        let dt = t - t0;
        if dt < MIN_DT_S {
            // Par degenerado: mantém `anterior` — a próxima amostra válida
            // cobre a janela inteira (sem update, sem invenção de potência).
            return Err(FalhaSensor::Inacessivel);
        }
        let delta_e = if energia >= e0 {
            energia - e0
        } else {
            // Wrap: e1 + range − e0 (range do contador, não inventado).
            let range = self.ler_uj("max_energy_range_uj")?;
            if range == 0 {
                return Err(FalhaSensor::Inacessivel);
            }
            range - e0 + energia
        };
        self.anterior = Some((t, energia));
        Ok(delta_e as f64 / 1e6 / dt)
    }

    fn descricao(&self) -> String {
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
    pub fn novo(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }
}

impl ActorDriver for RaplPowerCapActor {
    fn apply(&mut self, valor: &Value) -> Result<(), ErroAtor> {
        let Some(w) = valor.as_num() else {
            return Err(ErroAtor::ValorInvalido(format!(
                "CpuPowerCap espera valor numérico em W, recebeu {valor}"
            )));
        };
        if !(0.0..).contains(&w) {
            return Err(ErroAtor::ValorInvalido(format!("potência negativa: {w} W")));
        }
        let uw = format!("{}", (w * 1e6) as u64);
        escrever_endpoint(&self.file, &uw)
    }

    fn heartbeat(&mut self) -> bool {
        self.file.exists()
    }

    fn descricao(&self) -> String {
        format!("rapl_constraint:{}", self.file.display())
    }
}

/// `Ventoinha` — PWM via hwmon (`/sys/class/hwmon/hwmon*/pwmN`, 0–255).
pub struct HwmonPwmActor {
    file: PathBuf,
}

impl HwmonPwmActor {
    pub fn novo(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }
}

impl ActorDriver for HwmonPwmActor {
    fn apply(&mut self, valor: &Value) -> Result<(), ErroAtor> {
        let Some(v) = valor.as_num() else {
            return Err(ErroAtor::ValorInvalido(format!(
                "Ventoinha espera PWM numérico 0–255, recebeu {valor}"
            )));
        };
        if !(0.0..=255.0).contains(&v) || v.fract() != 0.0 {
            return Err(ErroAtor::ValorInvalido(format!(
                "PWM fora do domínio inteiro 0–255: {v}"
            )));
        }
        escrever_endpoint(&self.file, &format!("{}", v as u8))
    }

    fn heartbeat(&mut self) -> bool {
        self.file.exists()
    }

    fn descricao(&self) -> String {
        format!("hwmon_pwm:{}", self.file.display())
    }
}

/// `LedIndicador` — LED class (`/sys/class/leds/*/brightness`).
/// Estado textual do registro (§6): cores nomeadas → brilho via mapa de
/// configuração (`verde`/`vermelho`/`amarelo`/`azul`/`apagado` por padrão);
/// número direto é aceito como brilho (extensão honesta e auditável).
pub struct LedClassActor {
    dir: PathBuf,
    mapa: std::collections::BTreeMap<String, u8>,
    max: u8,
}

impl LedClassActor {
    pub fn novo(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let max = std::fs::read_to_string(dir.join("max_brightness"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .and_then(|m| u8::try_from(m).ok())
            .unwrap_or(255);
        let mut mapa = std::collections::BTreeMap::new();
        mapa.insert("apagado".to_string(), 0);
        mapa.insert("vermelho".to_string(), max);
        mapa.insert("verde".to_string(), (max / 4).max(1));
        mapa.insert("amarelo".to_string(), (max / 2).max(1));
        mapa.insert("azul".to_string(), (max / 3).max(1));
        Self { dir, mapa, max }
    }

    pub fn com_mapa(dir: impl Into<PathBuf>, mapa: std::collections::BTreeMap<String, u8>) -> Self {
        let mut d = Self::novo(dir);
        d.mapa = mapa;
        d
    }

    pub fn max_brilho(&self) -> u8 {
        self.max
    }
}

impl ActorDriver for LedClassActor {
    fn apply(&mut self, valor: &Value) -> Result<(), ErroAtor> {
        let brilho = match valor {
            Value::Str(s) | Value::Ident(s) => {
                self.mapa.get(s.as_str()).copied().ok_or_else(|| {
                    ErroAtor::ValorInvalido(format!(
                        "cor '{s}' fora do mapa do LedIndicador ({:?})",
                        self.mapa.keys().collect::<Vec<_>>()
                    ))
                })?
            }
            Value::Num(n) if (0.0..=255.0).contains(n) && n.fract() == 0.0 => *n as u8,
            Value::Num(n) => {
                return Err(ErroAtor::ValorInvalido(format!(
                    "brilho fora do domínio inteiro 0–255: {n}"
                )))
            }
        };
        if brilho > self.max {
            return Err(ErroAtor::ValorInvalido(format!(
                "brilho {brilho} excede max_brightness = {}",
                self.max
            )));
        }
        escrever_endpoint(&self.dir.join("brightness"), &format!("{brilho}"))
    }

    fn heartbeat(&mut self) -> bool {
        self.dir.join("brightness").exists()
    }

    fn descricao(&self) -> String {
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
pub fn descobrir(nome: &str) -> Option<crate::registry::Endpoint> {
    use crate::registry::Endpoint;
    match nome {
        "cpu_temp" => descobrir_thermal_zone().map(|dir| Endpoint::ThermalZone { dir }),
        "cpu_power" => descobrir_rapl_energy().map(|dir| Endpoint::RaplEnergy { dir }),
        "CpuPowerCap" => {
            let f = Path::new("/sys/class/powercap/intel-rapl:0/constraint_0_power_limit_uw");
            f.exists().then(|| Endpoint::RaplConstraint { file: f.into() })
        }
        "Ventoinha" => descobrir_pwm().map(|file| Endpoint::HwmonPwm { file }),
        "LedIndicador" => descobrir_led().map(|dir| Endpoint::LedClass { dir }),
        _ => None,
    }
}

/// Primeira thermal_zone plausível: preferência `x86_pkg_temp`/`cpu_*`;
/// caso nenhum `type` case, a primeira com `temp` legível.
pub fn descobrir_thermal_zone() -> Option<PathBuf> {
    let mut fallback = None;
    for zona in listar_dirs("/sys/class/thermal", "thermal_zone") {
        if !zona.join("temp").exists() {
            continue;
        }
        let tipo = std::fs::read_to_string(zona.join("type")).unwrap_or_default();
        let tipo = tipo.to_lowercase();
        if tipo.contains("x86_pkg_temp") || tipo.contains("cpu") || tipo.contains("acpitz") {
            return Some(zona);
        }
        fallback.get_or_insert(zona);
    }
    fallback
}

/// Primeiro domínio RAPL com `energy_uj`.
pub fn descobrir_rapl_energy() -> Option<PathBuf> {
    listar_dirs("/sys/class/powercap", "intel-rapl")
        .into_iter()
        .find(|d| d.join("energy_uj").exists())
}

/// Primeiro hwmon com PWM exportado.
pub fn descobrir_pwm() -> Option<PathBuf> {
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
pub fn descobrir_led() -> Option<PathBuf> {
    listar_dirs("/sys/class/leds", "").into_iter().find(|d| d.join("brightness").exists())
}

fn listar_dirs(base: &str, prefixo: &str) -> Vec<PathBuf> {
    let Ok(ent) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = ent
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            // Nome não-UTF8 só casa quando não há prefixo a respeitar (LEDs).
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(prefixo))
                    .unwrap_or(prefixo.is_empty())
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// Fábrica: endpoint → driver
// ---------------------------------------------------------------------------

/// Driver de sensor para endpoint real (ou `None` para endpoint sem driver
/// de leitura — `Simulado` é do bus, `Remote` é do transporte).
pub fn sensor_de(
    endpoint: &crate::registry::Endpoint,
) -> Option<Box<dyn SensorDriver + Send>> {
    use crate::registry::Endpoint;
    match endpoint {
        Endpoint::ThermalZone { dir } => Some(Box::new(ThermalZoneSensor::novo(dir.clone()))),
        Endpoint::HwmonTemp { file } => Some(Box::new(HwmonTempSensor::novo(file.clone()))),
        Endpoint::RaplEnergy { dir } => Some(Box::new(RaplEnergySensor::novo(dir.clone()))),
        _ => None,
    }
}

/// Driver de ator para endpoint real.
pub fn ator_de(endpoint: &crate::registry::Endpoint) -> Option<Box<dyn ActorDriver + Send>> {
    use crate::registry::Endpoint;
    match endpoint {
        Endpoint::RaplConstraint { file } => Some(Box::new(RaplPowerCapActor::novo(file.clone()))),
        Endpoint::HwmonPwm { file } => Some(Box::new(HwmonPwmActor::novo(file.clone()))),
        Endpoint::LedClass { dir } => Some(Box::new(LedClassActor::novo(dir.clone()))),
        _ => None,
    }
}
