//! Testes dos drivers reais contra árvores sysfs sintéticas em tmpdir — o
//! mesmo código de leitura/escrita que roda contra o hardware (integração
//! honesta em CI, PLAN §6.5). Unidades, wrap do RAPL, falhas honestas (§4.7)
//! e domínios de atuação.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use vbl_fxp::drivers::{
    actor_from, sensor_from, ActorDriver, AttentionSource, HwmonPwmActor, LedClassActor,
    RaplEnergySensor, RaplPowerCapActor, SensorDriver, SimulatedAttention, ThermalZoneSensor,
};
use vbl_fxp::registry::Endpoint;
use vbl_runtime::fxp::{SensorFailure, Value};

/// Tmpdir único (sem dependências externas).
fn tmpdir(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vbl-fxp-{}-{}-{}",
        name,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("criar tmpdir");
    dir
}

/// `thermal_zone` sintética: `temp` em mili°C.
#[test]
fn thermal_zone_converts_millicelsius_to_celsius() {
    let dir = tmpdir("tz");
    fs::write(dir.join("temp"), "45123").unwrap();
    fs::write(dir.join("type"), "x86_pkg_temp").unwrap();

    let mut s = ThermalZoneSensor::new(&dir);
    assert_eq!(s.read().unwrap(), 45.123);
    assert_eq!(s.description(), format!("thermal_zone:{}", dir.display()));

    // Sensor some (inacessível) — nunca vira 0.0 (§4.7).
    fs::remove_file(dir.join("temp")).unwrap();
    assert_eq!(s.read(), Err(SensorFailure::Inaccessible));

    // Conteúdo não numérico também é inacessível, nunca 0.0.
    fs::write(dir.join("temp"), "erro").unwrap();
    assert_eq!(s.read(), Err(SensorFailure::Inaccessible));
}

/// `hwmon_temp` sintético: `tempN_input` em mili°C (ex.: k10temp).
#[test]
fn hwmon_temp_converts_millicelsius_to_celsius() {
    let dir = tmpdir("hwmon_temp");
    let file = dir.join("temp1_input");
    fs::write(&file, "62500").unwrap();

    let mut s = sensor_from(&Endpoint::HwmonTemp { file: file.clone() })
        .expect("hwmon_temp deve fabricar sensor");
    assert_eq!(s.read().unwrap(), 62.5);
    assert_eq!(s.description(), format!("hwmon_temp:{}", file.display()));

    // Inacessível e não numérico — nunca 0.0 (§4.7).
    fs::remove_file(&file).unwrap();
    assert_eq!(s.read(), Err(SensorFailure::Inaccessible));
    fs::write(&file, "n/d").unwrap();
    assert_eq!(s.read(), Err(SensorFailure::Inaccessible));

    // Parse do endpoint no registro.
    assert_eq!(
        Endpoint::parse(&format!("hwmon_temp:{}", file.display())).unwrap(),
        Endpoint::HwmonTemp { file }
    );
}

/// RAPL consumo: série determinística com relógio injetado — primeira amostra
/// aquece; W = ΔE/Δt; wrap tratado com `max_energy_range_uj`.
#[test]
fn rapl_energy_deterministic_series_with_wrap() {
    let dir = tmpdir("rapl2");
    let queue: Vec<(f64, u64)> = vec![
        (0.0, 1_000_000),     // aquecimento
        (2.0, 3_000_000),     // ΔE = 2 J em 2 s → 1 W
        (4.0, 500_000),       // wrap: range 4 J → ΔE = range − e0 + e1 = 1.5 J em 2 s → 0.75 W
    ];
    let rest = queue.clone();
    let idx = AtomicUsize::new(0);
    let mut s = RaplEnergySensor::with_clock(
        &dir,
        Box::new(move || {
            let i = idx.fetch_add(1, Ordering::Relaxed).min(rest.len() - 1);
            rest[i].0
        }),
    );
    fs::write(dir.join("energy_uj"), "1000000").unwrap();
    fs::write(dir.join("max_energy_range_uj"), "4000000").unwrap();

    assert_eq!(s.read(), Err(SensorFailure::Inaccessible)); // aquecimento

    fs::write(dir.join("energy_uj"), "3000000").unwrap();
    let w = s.read().unwrap();
    assert!((w - 1.0).abs() < 1e-9, "esperado 1 W, obtido {w}");

    // Wrap do contador.
    fs::write(dir.join("energy_uj"), "500000").unwrap();
    let w = s.read().unwrap();
    assert!((w - 0.75).abs() < 1e-9, "esperado 0.75 W no wrap, obtido {w}");
}

/// Re-leitura degenerada (Δt < 1 ms, auditoria × avaliação no mesmo tick):
/// sem informação de potência — não sobrescreve a média válida anterior e
/// não fabrica W absurdos; a amostra válida seguinte cobre a janela inteira.
#[test]
fn rapl_energy_degenerate_pair_does_not_corrupt_power() {
    let dir = tmpdir("rapl3");
    let queue: Vec<(f64, u64)> = vec![
        (0.0, 1_000_000),        // aquecimento
        (0.000_001, 1_000_500),  // Δt = 1 µs — degenerado (ΔE = 500 µJ aqui NÃO vira W)
        (1.0, 1_020_000),        // par válido: Δt = 1 s desde a AQUECIMENTO, ΔE = 20 000 µJ
    ];
    let rest = queue.clone();
    let idx = AtomicUsize::new(0);
    let mut s = RaplEnergySensor::with_clock(
        &dir,
        Box::new(move || {
            let i = idx.fetch_add(1, Ordering::Relaxed).min(rest.len() - 1);
            rest[i].0
        }),
    );
    fs::write(dir.join("energy_uj"), "1000000").unwrap();

    assert_eq!(s.read(), Err(SensorFailure::Inaccessible)); // aquecimento

    fs::write(dir.join("energy_uj"), "1000500").unwrap();
    assert_eq!(s.read(), Err(SensorFailure::Inaccessible)); // degenerado: sem W

    fs::write(dir.join("energy_uj"), "1020000").unwrap();
    let w = s.read().unwrap();
    assert!((w - 0.02).abs() < 1e-9, "esperado 0.02 W (janela inteira), obtido {w}");
}

/// Relógio de parede: monotônico com resolução útil (regressão do bug do
/// `Instant::now()` fresco — Δt sempre ~0).
#[test]
fn wall_clock_advances() {
    use vbl_fxp::drivers::wall_clock;
    let clock = wall_clock();
    let t0 = clock();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let t1 = clock();
    assert!(t1 > t0, "relógio deve avançar ({t0} → {t1})");
    assert!(t1 - t0 >= 0.004, "avanço deve refletir a parede: {}", t1 - t0);
}

/// CpuPowerCap: comando em W → µW no sysfs.
#[test]
fn powercap_writes_microwatts_and_rejects_text() {
    let dir = tmpdir("cap");
    let file = dir.join("constraint_0_power_limit_uw");
    fs::write(&file, "250000000").unwrap();

    let mut a = RaplPowerCapActor::new(&file);
    assert!(a.heartbeat());

    a.apply(&Value::Num(50.0)).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "50000000");

    // Texto em ator numérico: InvalidValue (nunca entrega silenciosa).
    assert!(matches!(
        a.apply(&Value::Str("pouco".into())),
        Err(vbl_fxp::drivers::ActorError::InvalidValue(_))
    ));
    // Potência negativa: domínio inválido.
    assert!(matches!(
        a.apply(&Value::Num(-1.0)),
        Err(vbl_fxp::drivers::ActorError::InvalidValue(_))
    ));

    // Heartbeat falha quando o endpoint some (BDD Caso 3).
    fs::remove_file(&file).unwrap();
    assert!(!a.heartbeat());
    assert!(matches!(
        a.apply(&Value::Num(50.0)),
        Err(vbl_fxp::drivers::ActorError::WriteFailed(_))
    ));
}

/// Fan: PWM inteiro 0–255.
#[test]
fn pwm_writes_integer_and_rejects_out_of_domain() {
    let dir = tmpdir("pwm");
    let file = dir.join("pwm1");
    fs::write(&file, "0").unwrap();

    let mut a = HwmonPwmActor::new(&file);
    a.apply(&Value::Num(200.0)).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "200");

    for bad in [256.0, -1.0, 10.5] {
        assert!(
            matches!(
                a.apply(&Value::Num(bad)),
                Err(vbl_fxp::drivers::ActorError::InvalidValue(_))
            ),
            "PWM {bad} deveria ser rejeitado"
        );
    }
    assert!(matches!(
        a.apply(&Value::Ident("rapido".into())),
        Err(vbl_fxp::drivers::ActorError::InvalidValue(_))
    ));
}

/// StatusLed: cores nomeadas do registro §6 + max_brightness.
#[test]
fn led_resolves_colors_from_map_and_respects_max_brightness() {
    let dir = tmpdir("led");
    fs::write(dir.join("max_brightness"), "255").unwrap();
    fs::write(dir.join("brightness"), "0").unwrap();

    let mut a = LedClassActor::new(&dir);
    a.apply(&Value::Str("green".into())).unwrap();
    assert_eq!(fs::read_to_string(dir.join("brightness")).unwrap(), "63");

    a.apply(&Value::Ident("off".into())).unwrap();
    assert_eq!(fs::read_to_string(dir.join("brightness")).unwrap(), "0");

    // Cor fora do mapa: InvalidValue — sem fabricação.
    assert!(matches!(
        a.apply(&Value::Str("roxo".into())),
        Err(vbl_fxp::drivers::ActorError::InvalidValue(_))
    ));

    // max_brightness limitado reduz o domínio.
    let dir2 = tmpdir("led2");
    fs::write(dir2.join("max_brightness"), "10").unwrap();
    fs::write(dir2.join("brightness"), "0").unwrap();
    let mut a2 = LedClassActor::new(&dir2);
    assert_eq!(a2.max_brightness(), 10);
    assert!(matches!(
        a2.apply(&Value::Num(200.0)),
        Err(vbl_fxp::drivers::ActorError::InvalidValue(_))
    ));
    a2.apply(&Value::Num(7.0)).unwrap();
    assert_eq!(fs::read_to_string(dir2.join("brightness")).unwrap(), "7");
}

/// AttentionSource simulado (backend obrigatório em CI — PLAN §3.2).
#[test]
fn simulated_attention_is_default_source() {
    let mut a = SimulatedAttention { value_pct: 15.0 };
    assert_eq!(a.read().unwrap(), 15.0);
    a.value_pct = 300.0;
    assert_eq!(a.read().unwrap(), 100.0, "domínio 0–100%");
}

/// Fábrica endpoint → driver, e auto-descoberta roda sem pânico.
#[test]
fn fabrication_and_discovery_are_consistent() {
    let dir = tmpdir("fab");
    fs::write(dir.join("temp"), "50000").unwrap();
    let ep = Endpoint::ThermalZone { dir: dir.clone() };
    let mut d = sensor_from(&ep).expect("thermal_zone deve fabricar sensor");
    assert_eq!(d.read().unwrap(), 50.0);
    assert!(sensor_from(&Endpoint::Simulated).is_none());

    let file = dir.join("pwm9");
    fs::write(&file, "1").unwrap();
    let mut a = actor_from(&Endpoint::HwmonPwm { file }).expect("pwm deve fabricar ator");
    assert!(a.heartbeat());

    // Auto-descoberta: consistente em qualquer host (achar ou não é válido).
    for name in ["cpu_temp", "cpu_power", "CpuPowerCap", "Fan", "StatusLed", "x"] {
        let _ = vbl_fxp::drivers::discover(name);
    }
    // attention não é descobrível: simulado é o padrão obrigatório.
    assert!(vbl_fxp::drivers::discover("attention").is_none());
}

/// Leitura com path inexistente: Inacessivel honesto (§4.7).
#[test]
fn nonexistent_and_inaccessible_path() {
    let mut s = ThermalZoneSensor::new(Path::new("/nao/existe/tz"));
    assert_eq!(s.read(), Err(SensorFailure::Inaccessible));
    let mut s = RaplEnergySensor::new("/nao/existe/rapl");
    assert_eq!(s.read(), Err(SensorFailure::Inaccessible));
}

// ══════════════════════════════════════════════════════════════════════════
// Cobertura complementar: Display de ActorError, LED (mapa custom, domínio
// fracionário, teto de brilho), RAPL com contador range==0 e descrições.
// ══════════════════════════════════════════════════════════════════════════
use vbl_fxp::drivers::ActorError;

#[test]
fn display_de_actor_error() {
    assert_eq!(ActorError::WriteFailed("permissão".into()).to_string(), "escrita falhou: permissão");
    assert_eq!(ActorError::InvalidValue("fora".into()).to_string(), "valor inválido: fora");
}

#[test]
fn led_com_mapa_custom_dominio_fracionario_e_teto() {
    let dir = std::env::temp_dir().join(format!("vbl-led2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("max_brightness"), "100").unwrap();
    std::fs::write(dir.join("brightness"), "0").unwrap(); // sysfs: o arquivo existe

    let mut mapa = std::collections::BTreeMap::new();
    mapa.insert("verde".to_string(), 50u8);
    let mut led = LedClassActor::with_map(dir.clone(), mapa);
    assert_eq!(led.max_brightness(), 100);

    // brilho fracionário → fora do domínio inteiro
    assert!(matches!(
        led.apply(&Value::Num(1.5)),
        Err(ActorError::InvalidValue(m)) if m.contains("fora do domínio"),
    ));
    // cor fora do mapa → recusa sem falsificar
    assert!(led.apply(&Value::Str("roxo".into())).is_err());
    // brilho válido abaixo do teto grava
    assert!(led.apply(&Value::Num(30.0)).is_ok());
    assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap(), "30");
    // acima do max_brightness → recusa
    assert!(matches!(
        led.apply(&Value::Num(101.0)),
        Err(ActorError::InvalidValue(m)) if m.contains("max_brightness"),
    ));
    // heartbeat: brightness existe
    assert!(led.heartbeat());
}

#[test]
fn rapl_wrap_com_range_zero_nao_inventa_potencia() {
    let dir = std::env::temp_dir().join(format!("vbl-rapl2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("energy_uj"), "500").unwrap();
    std::fs::write(dir.join("max_energy_range_uj"), "0").unwrap(); // contador sem range
    let mut sensor = RaplEnergySensor::new(dir.clone());
    let _ = sensor.read().unwrap_err(); // amostra de aquecimento
    std::fs::write(dir.join("energy_uj"), "100").unwrap(); // wrap com range 0
    assert_eq!(sensor.read(), Err(SensorFailure::Inaccessible));
}

#[test]
fn descricoes_dos_drivers_reais() {
    let base = std::env::temp_dir().join(format!("vbl-desc-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let rotas: Vec<(vbl_fxp::registry::Endpoint, &str)> = vec![
        (
            vbl_fxp::registry::Endpoint::ThermalZone { dir: base.join("tz") },
            "thermal_zone",
        ),
        (
            vbl_fxp::registry::Endpoint::HwmonTemp { file: base.join("temp") },
            "hwmon_temp",
        ),
        (
            vbl_fxp::registry::Endpoint::RaplEnergy { dir: base.join("rapl") },
            "rapl_energy",
        ),
    ];
    for (endpoint, prefixo) in rotas {
        let driver = sensor_from(&endpoint).expect("driver de leitura");
        assert!(driver.description().starts_with(prefixo), "{}", driver.description());
    }
    let atores: Vec<(vbl_fxp::registry::Endpoint, &str)> = vec![
        (
            vbl_fxp::registry::Endpoint::RaplConstraint { file: base.join("cap") },
            "rapl_constraint",
        ),
        (
            vbl_fxp::registry::Endpoint::HwmonPwm { file: base.join("pwm") },
            "hwmon_pwm",
        ),
        (vbl_fxp::registry::Endpoint::LedClass { dir: base.join("led") }, "led:"),
    ];
    for (endpoint, prefixo) in atores {
        let driver = actor_from(&endpoint).expect("driver de atuação");
        assert!(driver.description().starts_with(prefixo), "{}", driver.description());
    }
}
