//! `vbl` — interpretador de console `.vl` (entregáveis das Etapas 2–4,
//! PLAN §2.3/§3/§4).
//!
//! Subcomandos:
//! - `vbl check <arquivo.vl>`: valida o programa (parser + registro FXP
//!   mínimo) e imprime diagnósticos com linha/coluna;
//! - `vbl run <arquivo.vl>`: carrega o estado inicial na memória e executa o
//!   loop de tick (relógio virtual por padrão; modo tempo real com
//!   `--real-ms`), com persistência `equilibrium` e Caderno auditável;
//! - `vbl fxp-probe`: audita o registro FXP do host (dispositivo × modo ×
//!   rota × disponibilidade × latência) e a cobertura dos dispositivos
//!   obrigatórios (FORMAL §6);
//! - `vbl ledger-verify ARQUIVO`: verificação EXTERNA do log do Caderno
//!   (binário `.vcad` ou JSONL) — recomputa a cadeia SHA-256 e emite o
//!   relatório de integridade, Joules e atuações (Etapa 4, PLAN §4.1).
//!
//! Backend FXP do `run` (PLAN Etapa 3):
//! - padrão: simulador determinístico em processo (paridade com a Etapa 2);
//! - `--fxp-config ARQUIVO [--fxp-mode MODO]`: barramento real (`FxpBus`)
//!   com registro rico, drivers reais (thermal_zone, RAPL, hwmon PWM, LED)
//!   e/ou peers remotos — dado sintético só circula em modo simulado/
//!   híbrido explícito, marcado no Caderno (FORMAL §4.7).
//!
//! Caderno do `run` (PLAN Etapa 4):
//! - sem `--ledger`: cadeia SHA-256 em memória ([`ChainLedger`], soma no
//!   final da execução);
//! - com `--ledger ARQUIVO`: Caderno de PRODUÇÃO — gravação assíncrona em
//!   buffer (thread dedicada), binário compacto `.vcad` em ARQUIVO e export
//!   JSONL em `ARQUIVO.jsonl`; a integridade é reavermelhada do arquivo ao
//!   final (agente externo: `vbl ledger-verify`).
//!
//! O loop assíncrono usa tokio (PLAN §2.2); o núcleo do engine é
//! determinístico (relógio virtual injetável) — a simulação roteirizada é
//! reproduzível tick a tick.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vbl_runtime::ledger::Ledger;
use vbl_runtime::production_ledger::{jsonl_from_binary, verify, ProductionLedger};
use vbl_runtime::json::Json;
use vbl_runtime::{load, validate, ChainLedger, Engine, FxpSimulator, MainInterpreter};

mod args;
mod script;

use args::{parse_args, Command};
use script::Script;
use vbl_fxp::registry::{DeviceKind, DeviceRegistry, Endpoint, FxpConfig, OperationMode, RemoteAddr};
use vbl_fxp::{BusConfig, FxpBus};

const MINIMUM_REGISTRY: &str = "\
registro mínimo do FXP (FORMAL §6):
  sensores : cpu_temp (temperatura, °C), cpu_power (potencia, W), attention (atencao, %)
  atores   : CpuPowerCap [10..250, safety 200], Fan [0..255, safety 200], StatusLed
";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    rt.block_on(async_main());
}

async fn async_main() {
    let cmd = match parse_args(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    match cmd {
        Command::Check { arquivo, with_registry } => check(&arquivo, with_registry),
        Command::Run {
            arquivo, ticks, real_ms, persist_dir, ledger, script, allow_unregistered,
            fxp_mode, fxp_config,
        } => match build_fxp(&fxp_config, &fxp_mode) {
            Ok((registry, config_bus)) => {
                // Barramento FXP real/híbrido/simulado configurado.
                let sim = script.build_simulator();
                let bus = FxpBus::build(registry, config_bus, sim);
                run(&arquivo, ticks, real_ms, persist_dir, ledger, script, allow_unregistered, bus).await
            }
            Err(_) => {
                // Sem `--fxp-config`/`--fxp-mode`: simulador em processo,
                // paridade exata com a Etapa 2 (bit a bit).
                let sim = script.build_simulator();
                run(&arquivo, ticks, real_ms, persist_dir, ledger, script, allow_unregistered, sim).await
            }
        },
        Command::FxpProbe { fxp_mode, fxp_config } => fxp_probe(&fxp_config, &fxp_mode),
        Command::LedgerVerify { arquivo } => ledger_verify(&arquivo),
    }
}

/// Resolve `--fxp-config`/`--fxp-mode` em (registro rico, config do bus).
/// `Err(())` = flags ausentes → backend simulado puro da Etapa 2.
fn build_fxp(
    fxp_config: &Option<PathBuf>,
    fxp_mode: &Option<String>,
) -> Result<(DeviceRegistry, BusConfig), ()> {
    if fxp_config.is_none() && fxp_mode.is_none() {
        return Err(());
    }
    let mut registry = DeviceRegistry::minimum();
    let mut cfg_fxp = None;
    if let Some(path) = fxp_config {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("vbl: não foi possível ler '{}': {e}", path.display());
            std::process::exit(2);
        });
        let cfg = FxpConfig::parse(&text).unwrap_or_else(|e| {
            eprintln!("vbl: config FXP inválida em '{}': {e}", path.display());
            std::process::exit(1);
        });
        cfg.apply(&mut registry).unwrap_or_else(|e| {
            eprintln!("vbl: registro FXP inválido: {e}");
            std::process::exit(1);
        });
        cfg_fxp = Some(cfg);
    }
    // Modo: flag > arquivo de config > simulado (default).
    let mode = match fxp_mode.as_deref() {
        Some(m) => OperationMode::parse(m).unwrap_or_else(|e| {
            eprintln!("vbl: {e}");
            std::process::exit(2);
        }),
        None => cfg_fxp.as_ref().and_then(|c| c.mode).unwrap_or(OperationMode::Simulated),
    };
    let mut config = BusConfig { mode, ..Default::default() };
    if let Some(c) = &cfg_fxp {
        if let Some(ms) = c.cache_ttl_ms {
            config.cache_ttl = Duration::from_millis(ms);
        }
        if let Some(ms) = c.read_timeout_ms {
            config.read_timeout = Duration::from_millis(ms);
        }
        if let Some(ms) = c.act_timeout_local_ms {
            config.act_timeout_local = Duration::from_millis(ms);
        }
        if let Some(ms) = c.act_timeout_remote_ms {
            config.act_timeout_remote = Duration::from_millis(ms);
        }
        if let Some(ms) = c.queue_timeout_ms {
            // O relógio virtual do engine é 1 tick = 1 s (FORMAL §2.1);
            // convertendo o prazo da fila de ms para ticks (mínimo 1).
            config.queue_timeout_ticks = ms.div_ceil(1000).max(1);
        }
        if let Some(r) = c.retries {
            config.retries = r;
        }
    }
    Ok((registry, config))
}

// ----------------------------------------------------------------------
// vbl check
// ----------------------------------------------------------------------
fn check(arquivo: &str, with_registry: bool) {
    let source = match std::fs::read_to_string(arquivo) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("vbl: não foi possível ler '{arquivo}': {e}");
            std::process::exit(2);
        }
    };
    let (_program, diags) = vbl_lang::parse(&source);
    let mut diagnosticos = diags.items.clone();
    if with_registry && !diagnosticos.iter().any(|d| d.is_error()) {
        // validação contra o registro mínimo (FORMAL §3/§6)
        let (program, _) = vbl_lang::parse(&source);
        let fxp = FxpSimulator::new();
        for d in validate(fxp.registry(), &program) {
            diagnosticos.push(vbl_lang::Diagnostic::error(&d.code, vbl_lang::Span::default(), d.message));
        }
    }
    diagnosticos.sort_by_key(|d| (d.span.line, d.span.col));
    if diagnosticos.is_empty() {
        println!("ok: {arquivo} — programa válido");
        return;
    }
    for d in &diagnosticos {
        println!("{d}");
    }
    let errors = diagnosticos.iter().filter(|d| d.is_error()).count();
    eprintln!("{arquivo}: {errors} erro(s) de compilação");
    std::process::exit(1);
}

// ----------------------------------------------------------------------
// vbl run (genérico no backend FXP e no Caderno: memória ou produção)
// ----------------------------------------------------------------------
#[allow(clippy::too_many_arguments)]
async fn run<F: vbl_runtime::fxp::Fxp>(
    arquivo: &str,
    ticks: Option<u64>,
    real_ms: Option<u64>,
    persist_dir: PathBuf,
    ledger: Option<PathBuf>,
    script: Script,
    allow_unregistered: bool,
    fxp: F,
) {
    let source = match std::fs::read_to_string(arquivo) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("vbl: não foi possível ler '{arquivo}': {e}");
            std::process::exit(2);
        }
    };
    let (program, diags) = vbl_lang::parse(&source);
    let errors: Vec<&vbl_lang::Diagnostic> = diags.errors().collect();
    if !errors.is_empty() {
        for d in &diags.items {
            println!("{d}");
        }
        eprintln!("vbl: {} erro(s) de compilação — programa não carregado", errors.len());
        std::process::exit(1);
    }

    // Validação contra o registro do backend (FORMAL §3/§6).
    if !allow_unregistered {
        let registry_diags = validate(fxp.registry(), &program);
        if !registry_diags.is_empty() {
            for d in &registry_diags {
                eprintln!("vbl: {d}");
            }
            eprintln!(
                "vbl: {} referência(ões) fora do registro do FXP — use --allow-unregistered para executar mesmo assim (falhas de I/O seguem FORMAL §4.7)",
                registry_diags.len()
            );
            eprintln!("{MINIMUM_REGISTRY}");
            std::process::exit(1);
        }
    }

    std::fs::create_dir_all(&persist_dir).expect("criar diretório de persistência");
    println!("▶ {arquivo} — relógio virtual 1 tick = 1s");

    match ledger {
        // Etapa 4 (PLAN §4.1): Caderno de produção — gravação assíncrona
        Some(binary) => {
            let production = ProductionLedger::open(&binary).unwrap_or_else(|e| {
                eprintln!("vbl: caderno '{}': {e}", binary.display());
                std::process::exit(2);
            });
            println!("  Caderno de produção: {} (assíncrono; JSONL em {})",
                binary.display(),
                jsonl_path(&binary).display());
            let mut engine = Engine::with_ledger(fxp, 1.0, &persist_dir, production);
            reload(&mut engine);
            let mut interp = load(&mut engine, &program);
            println!("  {} forma(s) carregada(s)", engine.active_names().len());
            let interval = real_ms.map(|ms| tokio::time::interval(Duration::from_millis(ms)));
            let start = Instant::now();
            let executed = run_loop(&mut engine, &mut interp, ticks.unwrap_or(u64::MAX), interval, &script).await;
            let duration = start.elapsed();
            let ativos: Vec<(String, String, String)> = engine
                .active_names()
                .iter()
                .filter_map(|n| {
                    engine.form(n).map(|f| {
                        (n.to_string(), format!("{}", f.value), f.conjugation.name().to_string())
                    })
                })
                .collect();
            // consumo do Caderno encerra a thread de gravação (fechar)
            let summary = engine.ledger.close().unwrap_or_else(|e| {
                eprintln!("vbl: {e}");
                std::process::exit(1);
            });
            run_summary(&ativos, executed, duration, Some(summary), Some(&binary));
        }
        // Sem --ledger: cadeia em memória (paridade com a Etapa 2)
        None => {
            let mut engine = Engine::new(fxp, 1.0, &persist_dir);
            reload(&mut engine);
            let mut interp = load(&mut engine, &program);
            println!("  {} forma(s) carregada(s)", engine.active_names().len());
            let interval = real_ms.map(|ms| tokio::time::interval(Duration::from_millis(ms)));
            let start = Instant::now();
            let executed = run_loop(&mut engine, &mut interp, ticks.unwrap_or(u64::MAX), interval, &script).await;
            let duration = start.elapsed();
            let ativos: Vec<(String, String, String)> = engine
                .active_names()
                .iter()
                .filter_map(|n| {
                    engine.form(n).map(|f| {
                        (n.to_string(), format!("{}", f.value), f.conjugation.name().to_string())
                    })
                })
                .collect();
            run_summary(&ativos, executed, duration, None, None);
            // sumário da cadeia em memória (implementação de referência)
            let events = engine.ledger.events.len();
            let leaks: f64 = engine
                .ledger
                .search("LEAK", &[])
                .iter()
                .filter_map(|e| match &e.extra {
                    Json::Obj(c) => c.get("joules").and_then(|j| match j {
                        Json::Num(n) => Some(*n),
                        _ => None,
                    }),
                    _ => None,
                })
                .sum();
            println!(
                "  Caderno (memória): {events} evento(s), {leaks:.2} J acumulados; cadeia SHA-256 {}",
                if engine.ledger.verify_chain() { "ÍNTEGRA" } else { "CORROMPIDA" }
            );
            println!("  cabeça da cadeia: {}…", &engine.ledger.chain_head()[..16]);
        }
    }
}

/// Recarga das `equilibrium` persistidas (FORMAL §4.1).
fn reload<C: Ledger, F: vbl_runtime::fxp::Fxp>(engine: &mut Engine<F, C>) -> usize {
    let n = vbl_runtime::persist::reload_equilibrium(engine);
    if n > 0 {
        println!("↺ {n} equilibrium recarregada(s) do suporte estável");
    }
    n
}

/// O loop de ticks (relógio virtual; tempo real opcional).
async fn run_loop<C: Ledger, F: vbl_runtime::fxp::Fxp>(
    engine: &mut Engine<F, C>,
    interp: &mut MainInterpreter,
    total: u64,
    mut interval: Option<tokio::time::Interval>,
    script: &Script,
) -> u64 {
    let mut executed: u64 = 0;
    for _ in 0..total {
        if let Some(iv) = &mut interval {
            iv.tick().await; // modo tempo real (1 tick = período do intervalo)
        }
        interp.run_due(engine);
        engine.tick();
        executed += 1;
        if engine.active_names().is_empty() && script.finished(engine.clock) {
            break;
        }
    }
    executed
}

/// Sumário comum dos dois caminhos de Caderno.
fn run_summary(
    ativos: &[(String, String, String)],
    executed: u64,
    duration: Duration,
    summary: Option<vbl_runtime::production_ledger::Summary>,
    binary: Option<&Path>,
) {
    println!(
        "■ {executed} tick(s) em {duration:.1?} — formas ativas restantes: {}",
        if ativos.is_empty() {
            "—".to_string()
        } else {
            const LIMIT: usize = 20;
            let summary: Vec<String> = ativos
                .iter()
                .take(LIMIT)
                .map(|(n, v, c)| format!("{n}: {v} ({c})"))
                .collect();
            if ativos.len() > LIMIT {
                format!("{} … (+{} formas)", summary.join(", "), ativos.len() - LIMIT)
            } else {
                summary.join(", ")
            }
        }
    );
    let (Some(summary), Some(binary)) = (summary, binary) else {
        return;
    };
    println!(
        "  Caderno de produção: {} evento(s), {} bytes, {:.2} J acumulados (gravação assíncrona)",
        summary.events, summary.bytes, summary.total_joules
    );
    // verificação EXTERNA: relê o arquivo e recompõe a cadeia
    let rel = verify(binary).unwrap_or_else(|e| {
        eprintln!("vbl: verificação do Caderno falhou: {e}");
        std::process::exit(1);
    });
    println!(
        "  cadeia SHA-256 {}: {} evento(s) no arquivo; atuações {}/{} ok; divergências (alertas): {}",
        if rel.chain_ok { "ÍNTEGRA" } else { "CORROMPIDA" },
        rel.events,
        rel.atuacoes_ok,
        rel.actuations,
        rel.alerts
    );
    println!("  cabeça da cadeia: {}…", &rel.chain_head[..16.min(rel.chain_head.len())]);
    let jsonl = jsonl_path(binary);
    match jsonl_from_binary(binary, &jsonl) {
        Ok(n) => println!("  log JSONL exportado para {} ({n} eventos)", jsonl.display()),
        Err(e) => eprintln!("vbl: conversão JSONL falhou: {e}"),
    }
    if !rel.chain_ok {
        eprintln!("vbl: log do Caderno CORROMPIDO — execução não passou na auditoria");
        std::process::exit(1);
    }
}

/// Caminho do export JSONL associado ao binário do Caderno.
fn jsonl_path(binary: &Path) -> PathBuf {
    let mut path = binary.as_os_str().to_owned();
    path.push(".jsonl");
    PathBuf::from(path)
}

// ----------------------------------------------------------------------
// vbl ledger-verify — verificação externa (AGENTS §1.4)
// ----------------------------------------------------------------------
fn ledger_verify(arquivo: &str) {
    let path = Path::new(arquivo);
    let rel = match verify(path) {
        Ok(rel) => rel,
        Err(e) => {
            eprintln!("vbl: {e}");
            std::process::exit(2);
        }
    };
    let format = if rel.footer_ok || path.extension().and_then(|e| e.to_str()) == Some("vcad") {
        "binário .vcad"
    } else {
        "JSONL"
    };
    println!("Caderno: {arquivo} ({format})");
    println!(
        "  cadeia SHA-256: {}",
        if rel.chain_ok {
            "ÍNTEGRA".to_string()
        } else {
            format!("CORROMPIDA (primeiro evento inválido: {:?})", rel.first_broken)
        }
    );
    println!("  eventos: {}; cabeça: {}…", rel.events, &rel.chain_head[..16.min(rel.chain_head.len())]);
    println!("  energia: {:.2} J acumulados", rel.total_joules);
    println!("  atuações: {}/{} com sucesso; divergências (alertas): {}", rel.atuacoes_ok, rel.actuations, rel.alerts);
    let mut counts: Vec<_> = rel.counts.iter().collect();
    counts.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (kind, n) in &counts {
        println!("    {kind}: {n}");
    }
    if !rel.chain_ok {
        std::process::exit(1);
    }
}

// ----------------------------------------------------------------------
// vbl fxp-probe
// ----------------------------------------------------------------------
fn fxp_probe(fxp_config: &Option<PathBuf>, fxp_mode: &Option<String>) {
    let (registry, config_bus) = match build_fxp(fxp_config, fxp_mode) {
        Ok(t) => t,
        Err(()) => (DeviceRegistry::minimum(), BusConfig::default()),
    };
    let mode_name = match config_bus.mode {
        OperationMode::Simulated => "simulado",
        OperationMode::Real => "real",
        OperationMode::Hybrid => "hibrido",
    };
    let mut bus = FxpBus::build(registry, config_bus, FxpSimulator::new());
    let mut ledger = ChainLedger::new();

    // Dados próprios (o probe precisa de &mut bus para as leituras).
    let devices: Vec<_> = bus
        .registry_rico()
        .devices()
        .map(|d| (d.name.clone(), d.kind.clone(), d.endpoint.clone()))
        .collect();
    println!("FXP — modo {mode_name} — {} dispositivo(s) no registro", devices.len());
    println!("{:<16} {:<26} {:<9} {:<34} disponibilidade", "dispositivo", "tipo", "unidade", "rota");
    let mut sensor_ok = 0usize;
    let mut sensores = 0usize;
    for (name, kind, endpoint) in &devices {
        let (kind_label, unit) = match kind {
            DeviceKind::Sensor { quantity, unit, precision_pct, .. } =>
                (format!("sensor {quantity} (±{precision_pct}%)"), unit.clone()),
            DeviceKind::Actor { limits } => {
                let mut t = "ator".to_string();
                if let (Some(min), Some(max)) = (limits.min, limits.max) {
                    t.push_str(&format!(" [{min}..{max}]"));
                }
                if let Some(s) = limits.safety_limit {
                    t.push_str(&format!(" safety {s}"));
                }
                (t, "—".to_string())
            }
        };
        let route = bus.route_of(name).map(|r| r.description()).unwrap_or_else(|| "—".into());
        let availability = match kind {
            DeviceKind::Sensor { .. } => {
                sensores += 1;
                let t0 = Instant::now();
                match vbl_runtime::fxp::Fxp::read_sensor(&mut bus, name, &mut ledger) {
                    Ok(v) => {
                        sensor_ok += 1;
                        format!("✓ {:.3} ({:?})", v, t0.elapsed())
                    }
                    Err(vbl_runtime::fxp::SensorFailure::Inaccessible) =>
                        "✗ inacessível (condição não avaliada — §4.7)".to_string(),
                    Err(vbl_runtime::fxp::SensorFailure::NotRegistered) => "✗ não registrado".to_string(),
                }
            }
            DeviceKind::Actor { .. } => actor_availability(endpoint),
        };
        println!("{:<16} {:<26} {:<9} {:<34} {}", name, kind_label, unit, route, availability);
    }
    println!(
        "sensores: {sensor_ok}/{sensores} acessíveis; alertas registrados no Caderno desta sonda: {}",
        ledger.search("ALERT", &[]).len()
    );

    // Cobertura dos dispositivos obrigatórios (FORMAL §6) — falha de CI se
    // faltar algo no denominador canônico.
    let mandatory = [
        ("cpu_temp", "sensor"), ("cpu_power", "sensor"), ("attention", "sensor"),
        ("CpuPowerCap", "ator"), ("Fan", "ator"), ("StatusLed", "ator"),
    ];
    let missing: Vec<String> = mandatory
        .iter()
        .filter(|(n, _)| !bus.registry_rico().contains(n))
        .map(|(n, k)| format!("{n} ({k})"))
        .collect();
    if missing.is_empty() {
        println!("cobertura obrigatória (§6): {}/{} ✓", mandatory.len(), mandatory.len());
    } else {
        println!("cobertura obrigatória (§6): {}/{} — faltando: {}",
            mandatory.len() - missing.len(), mandatory.len(),
            missing.join(", "));
        eprintln!("vbl: registro sem dispositivos obrigatórios (FORMAL §6)");
        std::process::exit(1);
    }
}

/// Disponibilidade de ator SEM atuar (probe é somente leitura): rota simulada
/// é sempre disponível; rota real confere a existência do endpoint; rota
/// remota confere o socket; inacessível reporta o motivo.
fn actor_availability(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Simulated => "✓ (sempre, simulado)".to_string(),
        Endpoint::Auto => "auto-descoberta no host (ver coluna rota)".to_string(),
        Endpoint::ThermalZone { dir }
        | Endpoint::RaplEnergy { dir }
        | Endpoint::LedClass { dir } => {
            if dir.exists() { "✓ endpoint presente".into() } else { "✗ endpoint ausente".into() }
        }
        Endpoint::RaplConstraint { file } | Endpoint::HwmonPwm { file } | Endpoint::HwmonTemp { file } => {
            if file.exists() { "✓ endpoint presente".into() } else { "✗ endpoint ausente".into() }
        }
        Endpoint::Remote { addr } => match addr {
            RemoteAddr::Unix(p) => {
                if p.exists() { "✓ socket presente".into() } else { "✗ socket ausente".into() }
            }
            RemoteAddr::Tcp { host, port } => {
                match format!("{host}:{port}").parse::<std::net::SocketAddr>() {
                    Ok(alvo) => match std::net::TcpStream::connect_timeout(&alvo, Duration::from_millis(500)) {
                        Ok(_) => "✓ peer alcançável".into(),
                        Err(e) => format!("✗ conexão falhou ({e})"),
                    },
                    Err(_) => format!("✗ endereço inválido ({host}:{port})"),
                }
            }
        },
    }
}
