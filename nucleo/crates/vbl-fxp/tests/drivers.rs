//! Testes dos drivers reais contra árvores sysfs sintéticas em tmpdir — o
//! mesmo código de leitura/escrita que roda contra o hardware (integração
//! honesta em CI, PLAN §6.5). Unidades, wrap do RAPL, falhas honestas (§4.7)
//! e domínios de atuação.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use vbl_fxp::drivers::{
    ator_de, sensor_de, ActorDriver, AttentionSource, HwmonPwmActor, LedClassActor,
    RaplEnergySensor, RaplPowerCapActor, SensorDriver, SimulatedAttention, ThermalZoneSensor,
};
use vbl_fxp::registry::Endpoint;
use vbl_runtime::fxp::{FalhaSensor, Value};

/// Tmpdir único (sem dependências externas).
fn tmpdir(nome: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vbl-fxp-{}-{}-{}",
        nome,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("criar tmpdir");
    dir
}

/// `thermal_zone` sintética: `temp` em mili°C.
#[test]
fn thermal_zone_converte_milicelsius_para_celsius() {
    let dir = tmpdir("tz");
    fs::write(dir.join("temp"), "45123").unwrap();
    fs::write(dir.join("type"), "x86_pkg_temp").unwrap();

    let mut s = ThermalZoneSensor::novo(&dir);
    assert_eq!(s.read().unwrap(), 45.123);
    assert_eq!(s.descricao(), format!("thermal_zone:{}", dir.display()));

    // Sensor some (inacessível) — nunca vira 0.0 (§4.7).
    fs::remove_file(dir.join("temp")).unwrap();
    assert_eq!(s.read(), Err(FalhaSensor::Inacessivel));

    // Conteúdo não numérico também é inacessível, nunca 0.0.
    fs::write(dir.join("temp"), "erro").unwrap();
    assert_eq!(s.read(), Err(FalhaSensor::Inacessivel));
}

/// RAPL consumo: série determinística com relógio injetado — primeira amostra
/// aquece; W = ΔE/Δt; wrap tratado com `max_energy_range_uj`.
#[test]
fn rapl_energy_serie_deterministica_com_wrap() {
    let dir = tmpdir("rapl2");
    let fila: Vec<(f64, u64)> = vec![
        (0.0, 1_000_000),     // aquecimento
        (2.0, 3_000_000),     // ΔE = 2 J em 2 s → 1 W
        (4.0, 500_000),       // wrap: range 4 J → ΔE = range − e0 + e1 = 1.5 J em 2 s → 0.75 W
    ];
    let resto = fila.clone();
    let idx = AtomicUsize::new(0);
    let mut s = RaplEnergySensor::com_relogio(
        &dir,
        Box::new(move || {
            let i = idx.fetch_add(1, Ordering::Relaxed).min(resto.len() - 1);
            resto[i].0
        }),
    );
    fs::write(dir.join("energy_uj"), "1000000").unwrap();
    fs::write(dir.join("max_energy_range_uj"), "4000000").unwrap();

    assert_eq!(s.read(), Err(FalhaSensor::Inacessivel)); // aquecimento

    fs::write(dir.join("energy_uj"), "3000000").unwrap();
    let w = s.read().unwrap();
    assert!((w - 1.0).abs() < 1e-9, "esperado 1 W, obtido {w}");

    // Wrap do contador.
    fs::write(dir.join("energy_uj"), "500000").unwrap();
    let w = s.read().unwrap();
    assert!((w - 0.75).abs() < 1e-9, "esperado 0.75 W no wrap, obtido {w}");
}

/// CpuPowerCap: comando em W → µW no sysfs.
#[test]
fn powercap_escreve_microwatts_e_rejeita_texto() {
    let dir = tmpdir("cap");
    let file = dir.join("constraint_0_power_limit_uw");
    fs::write(&file, "250000000").unwrap();

    let mut a = RaplPowerCapActor::novo(&file);
    assert!(a.heartbeat());

    a.apply(&Value::Num(50.0)).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "50000000");

    // Texto em ator numérico: ValorInvalido (nunca entrega silenciosa).
    assert!(matches!(
        a.apply(&Value::Str("pouco".into())),
        Err(vbl_fxp::drivers::ErroAtor::ValorInvalido(_))
    ));
    // Potência negativa: domínio inválido.
    assert!(matches!(
        a.apply(&Value::Num(-1.0)),
        Err(vbl_fxp::drivers::ErroAtor::ValorInvalido(_))
    ));

    // Heartbeat falha quando o endpoint some (BDD Caso 3).
    fs::remove_file(&file).unwrap();
    assert!(!a.heartbeat());
    assert!(matches!(
        a.apply(&Value::Num(50.0)),
        Err(vbl_fxp::drivers::ErroAtor::EscritaFalhou(_))
    ));
}

/// Ventoinha: PWM inteiro 0–255.
#[test]
fn pwm_escreve_inteiro_e_rejeita_fora_do_dominio() {
    let dir = tmpdir("pwm");
    let file = dir.join("pwm1");
    fs::write(&file, "0").unwrap();

    let mut a = HwmonPwmActor::novo(&file);
    a.apply(&Value::Num(200.0)).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "200");

    for ruim in [256.0, -1.0, 10.5] {
        assert!(
            matches!(
                a.apply(&Value::Num(ruim)),
                Err(vbl_fxp::drivers::ErroAtor::ValorInvalido(_))
            ),
            "PWM {ruim} deveria ser rejeitado"
        );
    }
    assert!(matches!(
        a.apply(&Value::Ident("rapido".into())),
        Err(vbl_fxp::drivers::ErroAtor::ValorInvalido(_))
    ));
}

/// LedIndicador: cores nomeadas do registro §6 + max_brightness.
#[test]
fn led_resolve_cores_do_mapa_e_respeita_max_brightness() {
    let dir = tmpdir("led");
    fs::write(dir.join("max_brightness"), "255").unwrap();
    fs::write(dir.join("brightness"), "0").unwrap();

    let mut a = LedClassActor::novo(&dir);
    a.apply(&Value::Str("verde".into())).unwrap();
    assert_eq!(fs::read_to_string(dir.join("brightness")).unwrap(), "63");

    a.apply(&Value::Ident("apagado".into())).unwrap();
    assert_eq!(fs::read_to_string(dir.join("brightness")).unwrap(), "0");

    // Cor fora do mapa: ValorInvalido — sem fabricação.
    assert!(matches!(
        a.apply(&Value::Str("roxo".into())),
        Err(vbl_fxp::drivers::ErroAtor::ValorInvalido(_))
    ));

    // max_brightness limitado reduz o domínio.
    let dir2 = tmpdir("led2");
    fs::write(dir2.join("max_brightness"), "10").unwrap();
    fs::write(dir2.join("brightness"), "0").unwrap();
    let mut a2 = LedClassActor::novo(&dir2);
    assert_eq!(a2.max_brilho(), 10);
    assert!(matches!(
        a2.apply(&Value::Num(200.0)),
        Err(vbl_fxp::drivers::ErroAtor::ValorInvalido(_))
    ));
    a2.apply(&Value::Num(7.0)).unwrap();
    assert_eq!(fs::read_to_string(dir2.join("brightness")).unwrap(), "7");
}

/// AttentionSource simulado (backend obrigatório em CI — PLAN §3.2).
#[test]
fn attention_simulada_e_a_fonte_padrao() {
    let mut a = SimulatedAttention { valor_pct: 15.0 };
    assert_eq!(a.read().unwrap(), 15.0);
    a.valor_pct = 300.0;
    assert_eq!(a.read().unwrap(), 100.0, "domínio 0–100%");
}

/// Fábrica endpoint → driver, e auto-descoberta roda sem pânico.
#[test]
fn fabrica_e_descoberta_sao_consistentes() {
    let dir = tmpdir("fab");
    fs::write(dir.join("temp"), "50000").unwrap();
    let ep = Endpoint::ThermalZone { dir: dir.clone() };
    let mut d = sensor_de(&ep).expect("thermal_zone deve fabricar sensor");
    assert_eq!(d.read().unwrap(), 50.0);
    assert!(sensor_de(&Endpoint::Simulado).is_none());

    let file = dir.join("pwm9");
    fs::write(&file, "1").unwrap();
    let mut a = ator_de(&Endpoint::HwmonPwm { file }).expect("pwm deve fabricar ator");
    assert!(a.heartbeat());

    // Auto-descoberta: consistente em qualquer host (achar ou não é válido).
    for nome in ["cpu_temp", "cpu_power", "CpuPowerCap", "Ventoinha", "LedIndicador", "x"] {
        let _ = vbl_fxp::drivers::descobrir(nome);
    }
    // attention não é descobrível: simulado é o padrão obrigatório.
    assert!(vbl_fxp::drivers::descobrir("attention").is_none());
}

/// Leitura com path inexistente: Inacessivel honesto (§4.7).
#[test]
fn path_inexistente_e_inacessivel() {
    let mut s = ThermalZoneSensor::novo(Path::new("/nao/existe/tz"));
    assert_eq!(s.read(), Err(FalhaSensor::Inacessivel));
    let mut s = RaplEnergySensor::novo("/nao/existe/rapl");
    assert_eq!(s.read(), Err(FalhaSensor::Inacessivel));
}
