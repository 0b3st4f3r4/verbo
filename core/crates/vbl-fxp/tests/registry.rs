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

// ══════════════════════════════════════════════════════════════════════════
// Cobertura complementar: modos, endpoints, matriz de chaves da config,
// apply (overrides + extensões) e Display dos erros.
// ══════════════════════════════════════════════════════════════════════════
use vbl_fxp::registry::RemoteAddr;

#[test]
fn modos_parse_nome_e_erro() {
    // modo do barramento
    assert_eq!(OperationMode::parse("real").unwrap(), OperationMode::Real);
    assert_eq!(OperationMode::parse("simulado").unwrap(), OperationMode::Simulated);
    assert_eq!(OperationMode::parse("hibrido").unwrap(), OperationMode::Hybrid);
    assert_eq!(OperationMode::Real.name(), "real");
    assert_eq!(OperationMode::Simulated.name(), "simulado");
    assert_eq!(OperationMode::Hybrid.name(), "hibrido");
    assert!(OperationMode::parse("voador").is_err());
    // modo individual do dispositivo
    assert_eq!(DeviceMode::parse("real").unwrap(), DeviceMode::Real);
    assert_eq!(DeviceMode::parse("simulado").unwrap(), DeviceMode::Simulated);
    assert_eq!(DeviceMode::Real.name(), "real");
    assert_eq!(DeviceMode::Simulated.name(), "simulado");
    assert_eq!(DeviceMode::default(), DeviceMode::Simulated);
    assert!(DeviceMode::parse("hibrido").is_err()); // modo de dispositivo não herda o nome do global
}

#[test]
fn endpoint_parse_erro_e_descricao_de_todas_as_rotas() {
    // tcp malformado
    let e = Endpoint::parse("tcp:sem_porta").unwrap_err().to_string();
    assert!(e.contains("esperado tcp:host:porta"), "{e}");
    let e = Endpoint::parse("tcp:127.0.0.1:porta").unwrap_err().to_string();
    assert!(e.contains("porta inválida"), "{e}");
    // esquema desconhecido
    assert!(Endpoint::parse("bluetooth:x").is_err());

    // descrição canônica de cada rota (re-parseável)
    let casos = [
        ("simulado", Endpoint::Simulated),
        ("auto", Endpoint::Auto),
        ("thermal_zone:/sys/t0", Endpoint::ThermalZone { dir: "/sys/t0".into() }),
        ("hwmon_temp:/sys/hw", Endpoint::HwmonTemp { file: "/sys/hw".into() }),
        ("rapl_energy:/sys/rapl", Endpoint::RaplEnergy { dir: "/sys/rapl".into() }),
        ("rapl_constraint:/sys/c", Endpoint::RaplConstraint { file: "/sys/c".into() }),
        ("hwmon_pwm:/sys/pwm", Endpoint::HwmonPwm { file: "/sys/pwm".into() }),
        ("led:/sys/leds", Endpoint::LedClass { dir: "/sys/leds".into() }),
        ("unix:/tmp/fxpd.sock", Endpoint::Remote { addr: RemoteAddr::Unix("/tmp/fxpd.sock".into()) }),
        ("tcp:127.0.0.1:9000", Endpoint::Remote { addr: RemoteAddr::Tcp { host: "127.0.0.1".into(), port: 9000 } }),
    ];
    for (texto, endpoint) in casos {
        assert_eq!(Endpoint::parse(texto).unwrap(), endpoint, "{texto}");
        assert_eq!(endpoint.description(), texto, "{texto:?}");
    }
    // só rotas remotas são remotas
    assert!(Endpoint::Remote { addr: RemoteAddr::Unix("/x".into()) }.is_remote());
    assert!(!Endpoint::Simulated.is_remote());
    // registro: contagem e vazio
    let vazio = DeviceRegistry::default();
    assert!(vazio.is_empty());
    assert_eq!(DeviceRegistry::minimum().len(), 6); // 3 sensores + 3 atores (§6)
}

#[test]
fn config_matriz_de_chaves_globais_e_erros() {
    let cfg = FxpConfig::parse("\
# comentário
mode = hibrido
cache_ttl_ms = 100
read_timeout_ms = 20
act_timeout_local_ms = 40
act_timeout_remote_ms = 400
queue_timeout_ms = 2500
retries = 3
").unwrap();
    assert_eq!(cfg.mode, Some(OperationMode::Hybrid));
    assert_eq!(cfg.cache_ttl_ms, Some(100));
    assert_eq!(cfg.read_timeout_ms, Some(20));
    assert_eq!(cfg.act_timeout_local_ms, Some(40));
    assert_eq!(cfg.act_timeout_remote_ms, Some(400));
    assert_eq!(cfg.queue_timeout_ms, Some(2500));
    assert_eq!(cfg.retries, Some(3));

    // cláusulas de erro das chaves globais
    let e = FxpConfig::parse("sem igual").unwrap_err().to_string();
    assert!(e.contains("sem '='"), "{e}");
    let e = FxpConfig::parse("cache_ttl_ms = x").unwrap_err().to_string();
    assert!(e.contains("cache_ttl_ms espera inteiro"), "{e}");
    let e = FxpConfig::parse("act_timeout_local_ms = x").unwrap_err().to_string();
    assert!(e.contains("act_timeout_local_ms espera inteiro"), "{e}");
    let e = FxpConfig::parse("queue_timeout_ms = dez").unwrap_err().to_string();
    assert!(e.contains("queue_timeout_ms espera inteiro"), "{e}");
    let e = FxpConfig::parse("mode = voador").unwrap_err().to_string();
    assert!(e.contains("modo desconhecido"), "{e}");
    let e = FxpConfig::parse(format!("retries = {}", u64::from(u32::MAX) + 1).as_str())
        .unwrap_err().to_string();
    assert!(e.contains("retries muito grande"), "{e}");
    let e = FxpConfig::parse("chave_sem_ponto = 1").unwrap_err().to_string();
    assert!(e.contains("chave desconhecida 'chave_sem_ponto'"), "{e}");
    let e = FxpConfig::parse("fallback.Fan = , ,").unwrap_err().to_string();
    assert!(e.contains("fallback sem alternativos"), "{e}");
}

#[test]
fn config_matriz_de_campos_de_dispositivo() {
    let cfg = FxpConfig::parse("\
cpu_temp.mode = real
cpu_temp.endpoint = auto
cpu_temp.grandeza = temperatura
cpu_temp.unidade = °C
cpu_temp.precisao_pct = 2.5
cpu_temp.min = -40
cpu_temp.max = 120
Fan.safety_limit = 200
human_attention.alias_de = attention
ReserveFan.endpoint = unix:/tmp/fxpd.sock
").unwrap();
    let ct = &cfg.devices["cpu_temp"];
    assert_eq!(ct.mode, Some(DeviceMode::Real));
    assert_eq!(ct.endpoint, Some(Endpoint::Auto));
    assert_eq!(ct.quantity.as_deref(), Some("temperatura"));
    assert_eq!(ct.unit.as_deref(), Some("°C"));
    assert_eq!(ct.precision_pct, Some(2.5));
    assert_eq!(ct.min, Some(-40.0));
    assert_eq!(ct.max, Some(120.0));
    assert_eq!(cfg.devices["Fan"].safety_limit, Some(200.0));
    assert_eq!(cfg.devices["human_attention"].alias_of.as_deref(), Some("attention"));

    // erros por campo
    for (linha, trecho) in [
        ("cpu_temp.mode = voador", "modo de dispositivo desconhecido"),
        ("cpu_temp.endpoint = bluetooth:x", "endpoint inválido"),
        ("cpu_temp.precisao_pct = preciso", "precisao_pct espera número"),
        ("cpu_temp.min = frio", "min espera número"),
        ("Fan.safety_limit = muito", "safety_limit espera número"),
        ("cpu_temp.campo_novo = 1", "campo de dispositivo desconhecido 'campo_novo'"),
    ] {
        let e = FxpConfig::parse(linha).unwrap_err().to_string();
        assert!(e.contains(trecho), "{linha}: {e}");
    }
}

#[test]
fn apply_sobrescreve_existente_e_registra_extensoes() {
    let cfg = FxpConfig::parse("\
cpu_temp.mode = real
cpu_temp.grandeza = temperatura
cpu_temp.unidade = °C
cpu_temp.precisao_pct = 1.5
cpu_temp.min = -20
cpu_temp.max = 110
Fan.min = 5
Fan.max = 250
Fan.safety_limit = 180
human_attention.alias_de = attention
solar_panel.grandeza = potencia
solar_panel.unidade = W
solar_panel.min = 0
solar_panel.max = 500
solar_panel.precisao_pct = 3
ReserveFan.min = 0
ReserveFan.max = 255
RemoteLed.endpoint = unix:/tmp/led.sock
fallback.Fan = ReserveFan
").unwrap();
    let mut registry = DeviceRegistry::minimum();
    cfg.apply(&mut registry).unwrap();

    assert!(registry.contains("solar_panel")); // extensão sensor
    assert!(registry.contains("ReserveFan")); // extensão ator por limites
    assert!(registry.contains("RemoteLed")); // ator remoto sem limites
    assert!(registry.contains("human_attention")); // alias visível
    // fallback registrado: cita canônico do mínimo (alternativa válida)
    let cfg_fallback = FxpConfig::parse("fallback.Fan = CpuPowerCap").unwrap();
    assert!(cfg_fallback.apply(&mut DeviceRegistry::minimum()).is_ok());
    // fallback citando desconhecido → erro de registro (§4.3)
    let cfg_ruim = FxpConfig::parse("fallback.Fan = NemExiste").unwrap();
    let e = cfg_ruim.apply(&mut DeviceRegistry::minimum()).unwrap_err().to_string();
    assert!(e.contains("fora do registro"), "{e}");

    // erros de aplicação
    let mut r = DeviceRegistry::minimum();
    let e = FxpConfig::parse("cpu_temp.safety_limit = 100").unwrap()
        .apply(&mut r).unwrap_err().to_string();
    assert!(e.contains("sensor 'cpu_temp' não aceita safety_limit"), "{e}");
    let e = FxpConfig::parse("Fan.grandeza = potencia").unwrap()
        .apply(&mut r).unwrap_err().to_string();
    assert!(e.contains("ator 'Fan' não aceita grandeza"), "{e}");
    let e = FxpConfig::parse("novo_dispositivo.mode = simulado").unwrap()
        .apply(&mut r).unwrap_err().to_string();
    assert!(e.contains("precisa de grandeza (sensor) ou limites (ator)"), "{e}");
    let e = FxpConfig::parse("novo_sensor.grandeza = x\nnovo_sensor.safety_limit = 9").unwrap()
        .apply(&mut r).unwrap_err().to_string();
    assert!(e.contains("não aceita safety_limit"), "{e}");
    // alias encadeado e alias com mode/endpoint: rejeitados na leitura
    let e = FxpConfig::parse("a.alias_de = attention\nb.alias_de = a")
        .unwrap_err().to_string();
    assert!(e.contains("não pode apontar para outro alias"), "{e}");
    let e = FxpConfig::parse("humido.alias_de = attention\nhumido.mode = real")
        .unwrap_err().to_string();
    assert!(e.contains("não aceita mode/endpoint"), "{e}");
    // alias para canônico desconhecido
    let e = FxpConfig::parse("x.alias_de = nem_existe").unwrap()
        .apply(&mut r).unwrap_err().to_string();
    assert!(e.contains("aponta para dispositivo inexistente"), "{e}");
}

#[test]
fn display_de_todos_os_erros_de_registro() {
    assert_eq!(
        RegistryError::DuplicateName("Fan".into()).to_string(),
        "nome canônico 'Fan' já registrado"
    );
    assert_eq!(
        RegistryError::ConflictingAlias("ReserveFan".into()).to_string(),
        "'ReserveFan' colide com nome/alias já registrado"
    );
    assert_eq!(
        RegistryError::UnknownAlias { alias: "x".into(), canonical: "y".into() }.to_string(),
        "alias 'x' aponta para dispositivo inexistente 'y'"
    );
    assert_eq!(
        RegistryError::UnknownFallback { actor: "Fan".into(), alternativo: "N".into() }.to_string(),
        "fallback de 'Fan' cita 'N', fora do registro (FORMAL §4.3)"
    );
    assert_eq!(
        RegistryError::ChainedAlias("a".into()).to_string(),
        "alias 'a' não pode apontar para outro alias"
    );
    assert_eq!(
        RegistryError::InvalidEndpoint("x".into()).to_string(),
        "endpoint inválido: 'x' (simulado | auto | thermal_zone:… | rapl_energy:… | rapl_constraint:… | hwmon_pwm:… | led:… | unix:… | tcp:host:porta)"
    );
    assert_eq!(
        RegistryError::InvalidConfig("linha 3: tudo errado".into()).to_string(),
        "config inválida: linha 3: tudo errado"
    );
}
