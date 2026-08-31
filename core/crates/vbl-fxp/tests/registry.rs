//! Testes do registro v2: aliases (FORMAL §6), unicidade, projeção no
//! runtime, e o config de linhas `key = value` com fallback (§4.3).

use vbl_fxp::registry::{
    DeviceEntry, DeviceKind, DeviceMode, DeviceRegistry, Endpoint, RegistryError, FxpConfig,
    OperationMode,
};
use vbl_runtime::fxp::ActorLimits;

#[test]
fn minimum_registry_has_formal_6_mandatory_devices() {
    let r = DeviceRegistry::minimum();
    assert_eq!(r.len(), 6, "3 sensores + 3 atores obrigatórios");
    // Sensores: grandeza/unidade/precisão do §6.
    let t = r.get("cpu_temp").unwrap();
    let DeviceKind::Sensor { quantity, unit, precision_pct, .. } = &t.kind else {
        panic!("cpu_temp deveria ser sensor");
    };
    assert_eq!(quantity, "temperature");
    assert_eq!(unit, "°C");
    assert_eq!(*precision_pct, 2.0);
    let p = r.get("cpu_power").unwrap();
    let DeviceKind::Sensor { precision_pct, .. } = &p.kind else { panic!() };
    assert_eq!(*precision_pct, 5.0);
    // Atores: limites inclusivos do §6.
    let DeviceKind::Actor { limits } = &r.get("CpuPowerCap").unwrap().kind else { panic!() };
    assert_eq!(
        limits,
        &ActorLimits { min: Some(10.0), max: Some(250.0), safety_limit: Some(200.0) }
    );
    let DeviceKind::Actor { limits } = &r.get("Fan").unwrap().kind else { panic!() };
    assert_eq!(limits.safety_limit, Some(200.0));
    // Tudo simulado por padrão (CI-safe).
    assert!(r.devices().all(|d| d.mode == DeviceMode::Simulated));
}

#[test]
fn alias_resolves_to_canonical_read_identical() {
    let mut r = DeviceRegistry::minimum();
    r.set_alias("human_attention", "attention").unwrap();

    // Resolução O(1) e transparente.
    assert_eq!(r.canonical_of("human_attention"), "attention");
    assert_eq!(r.canonical_of("attention"), "attention");
    let by_alias = r.get("human_attention").unwrap();
    let by_canonical = r.get("attention").unwrap();
    assert_eq!(by_alias.name, by_canonical.name);

    // A projeção no runtime inclui o alias — o loader valida pelos dois nomes.
    let rt = r.to_runtime_registry();
    assert!(rt.sensores.contains_key("attention"));
    assert!(rt.sensores.contains_key("human_attention"));
}

#[test]
fn alias_rejects_collision_cycle_and_unknown() {
    let mut r = DeviceRegistry::minimum();
    r.set_alias("human_attention", "attention").unwrap();

    // Colisão com nome canônico.
    assert_eq!(
        r.set_alias("cpu_temp", "attention"),
        Err(RegistryError::ConflictingAlias("cpu_temp".into()))
    );
    // Colisão com alias existente.
    assert_eq!(
        r.set_alias("human_attention", "cpu_temp"),
        Err(RegistryError::ConflictingAlias("human_attention".into()))
    );
    // Alias de alias (encadeado) — inválido.
    assert_eq!(
        r.set_alias("atencao_humana", "human_attention"),
        Err(RegistryError::ChainedAlias("human_attention".into()))
    );
    // Canonical inexistente.
    assert!(matches!(
        r.set_alias("x", "nao_existe"),
        Err(RegistryError::UnknownAlias { .. })
    ));
    // Nome canônico duplicado.
    assert!(matches!(
        r.register(DeviceEntry::sensor("cpu_temp", "x", "y", 0.0)),
        Err(RegistryError::DuplicateName(_))
    ));
}

#[test]
fn config_parses_and_applies_overrides_and_extensions() {
    let cfg = FxpConfig::parse(
        "# exemplo da doc\n\
         mode = hibrido\n\
         cache_ttl_ms = 100\n\
         read_timeout_ms = 7\n\
         retries = 2\n\
         cpu_temp.mode = real\n\
         cpu_temp.endpoint = thermal_zone:/sys/class/thermal/thermal_zone0\n\
         human_attention.alias_de = attention\n\
         ReserveFan.mode = real\n\
         ReserveFan.endpoint = unix:/tmp/fxpd.sock\n\
         ReserveFan.min = 0\n\
         ReserveFan.max = 255\n\
         fallback.Fan = ReserveFan\n",
    )
    .unwrap();

    assert_eq!(cfg.mode, Some(OperationMode::Hybrid));
    assert_eq!(cfg.cache_ttl_ms, Some(100));
    assert_eq!(cfg.read_timeout_ms, Some(7));
    assert_eq!(cfg.retries, Some(2));

    let mut r = DeviceRegistry::minimum();
    cfg.apply(&mut r).unwrap();

    // Override do obrigatório.
    let t = r.get("cpu_temp").unwrap();
    assert_eq!(t.mode, DeviceMode::Real);
    assert_eq!(
        t.endpoint,
        Endpoint::ThermalZone { dir: "/sys/class/thermal/thermal_zone0".into() }
    );

    // Alias aplicado.
    assert!(r.contains("human_attention"));

    // Extensão nova: ator remoto com limites.
    let res = r.get("ReserveFan").unwrap();
    assert_eq!(res.mode, DeviceMode::Real);
    assert!(res.endpoint.is_remote());
    let DeviceKind::Actor { limits } = &res.kind else { panic!("esperava ator") };
    assert_eq!(limits.min, Some(0.0));
    assert_eq!(limits.max, Some(255.0));

    // Fallback no registro (FORMAL §4.3).
    assert_eq!(r.get("Fan").unwrap().fallback, vec!["ReserveFan".to_string()]);
}

#[test]
fn config_rejects_malformations() {
    let mut r = DeviceRegistry::minimum();
    // Sem '='.
    assert!(matches!(
        FxpConfig::parse("mode hibrido\n"),
        Err(RegistryError::InvalidConfig(_))
    ));
    // Modo desconhecido.
    assert!(matches!(
        FxpConfig::parse("mode turbo\n"),
        Err(RegistryError::InvalidConfig(_))
    ));
    // Endpoint desconhecido.
    assert!(matches!(
        FxpConfig::parse("cpu_temp.endpoint = gpio4:x\n"),
        Err(RegistryError::InvalidEndpoint(_))
    ));
    // Alias com endpoint.
    assert!(matches!(
        FxpConfig::parse("human_attention.alias_de = attention\nhuman_attention.endpoint = auto\n")
            .and_then(|c| c.apply(&mut r)),
        Err(RegistryError::InvalidConfig(_))
    ));
    // Fallback de sensor.
    assert!(matches!(
        FxpConfig::parse("fallback.cpu_temp = attention\n").and_then(|c| c.apply(&mut r)),
        Err(RegistryError::InvalidConfig(_))
    ));
    // Fallback para ator inexistente (FORMAL §4.3).
    assert!(matches!(
        FxpConfig::parse("fallback.Fan = Fantasma\n").and_then(|c| c.apply(&mut r)),
        Err(RegistryError::UnknownFallback { .. })
    ));
    // Fallback de ator fora do registro.
    assert!(matches!(
        FxpConfig::parse("fallback.Fantasma = Fan\n").and_then(|c| c.apply(&mut r)),
        Err(RegistryError::InvalidConfig(_))
    ));
    // Dispositivo novo sem se declarar.
    assert!(matches!(
        FxpConfig::parse("SolarPanel.mode = real\n").and_then(|c| c.apply(&mut r)),
        Err(RegistryError::InvalidConfig(_))
    ));
    // Sensor não aceita safety_limit (exclusivo de ator).
    assert!(matches!(
        FxpConfig::parse("SolarPanel.grandeza = luz\nSolarPanel.safety_limit = 10\n")
            .and_then(|c| c.apply(&mut r)),
        Err(RegistryError::InvalidConfig(_))
    ));
    // Ator não aceita metadados de sensor.
    assert!(matches!(
        FxpConfig::parse("Fan.grandeza = fluxo\n").and_then(|c| c.apply(&mut r)),
        Err(RegistryError::InvalidConfig(_))
    ));
    // …mas min/max de sensor é faixa legítima (FORMAL §6: cpu_temp 0–120).
    let mut r2 = DeviceRegistry::minimum();
    FxpConfig::parse("SolarPanel.grandeza = luz\nSolarPanel.unidade = W/m2\nSolarPanel.min = 0\nSolarPanel.max = 1200\n")
        .and_then(|c| c.apply(&mut r2))
        .unwrap();
    let DeviceKind::Sensor { range: (min, max), .. } = &r2.get("SolarPanel").unwrap().kind
    else {
        panic!("esperava sensor");
    };
    assert_eq!((*min, *max), (Some(0.0), Some(1200.0)));
}

#[test]
fn text_endpoints_roundtrip() {
    for s in [
        "simulado",
        "auto",
        "thermal_zone:/sys/class/thermal/thermal_zone0",
        "rapl_energy:/sys/class/powercap/intel-rapl:0",
        "rapl_constraint:/sys/class/powercap/intel-rapl:0/constraint_0_power_limit_uw",
        "hwmon_pwm:/sys/class/hwmon/hwmon2/pwm1",
        "led:/sys/class/leds/input0::scroll",
        "unix:/tmp/fxpd.sock",
        "tcp:127.0.0.1:7777",
    ] {
        let ep = Endpoint::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"));
        assert_eq!(ep.description(), s, "descrição deve ser o formato canônico");
    }
    // tcp com porta inválida.
    assert!(matches!(
        Endpoint::parse("tcp:host:abc"),
        Err(RegistryError::InvalidEndpoint(_))
    ));
    // Sem esquema.
    assert!(matches!(
        Endpoint::parse("/sys/class/thermal"),
        Err(RegistryError::InvalidEndpoint(_))
    ));
}
