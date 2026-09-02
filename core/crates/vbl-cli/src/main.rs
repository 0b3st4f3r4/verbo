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
//!
//! Cada comando devolve o código de saída (`dispatch(args) -> i32`) em vez
//! de chamar `process::exit` espalhado — o `main` é o único ponto de saída e
//! a suíte (`#[cfg(test)]` abaixo) ensaia os comandos in-process.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vbl_runtime::json::Json;
use vbl_runtime::ledger::Ledger;
use vbl_runtime::production_ledger::{jsonl_from_binary, verify, ProductionLedger};
use vbl_runtime::{load, validate, ChainLedger, Engine, FxpSimulator, MainInterpreter};

mod args;
mod script;

use args::{parse_args, Command};
use script::Script;
use vbl_fxp::registry::{
    DeviceKind, DeviceRegistry, Endpoint, FxpConfig, OperationMode, RemoteAddr,
};
use vbl_fxp::{BusConfig, FxpBus};

const MINIMUM_REGISTRY: &str = "\
registro mínimo do FXP (FORMAL §6):
  sensores : cpu_temp (temperatura, °C), cpu_power (potencia, W), attention (atencao, %)
  atores   : CpuPowerCap [10..250, safety 200], Fan [0..255, safety 200], StatusLed
";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let code = rt.block_on(dispatch(std::env::args().skip(1)));
    std::process::exit(code);
}

/// Roteia o subcomando e devolve o código de saída do processo
/// (0 ok · 1 erro de programa/auditoria · 2 uso ou I/O).
async fn dispatch<I>(args: I) -> i32
where
    I: Iterator<Item = String>,
{
    let cmd = match parse_args(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match cmd {
        Command::Check {
            arquivo,
            with_registry,
        } => check(&arquivo, with_registry),
        Command::Run {
            arquivo,
            ticks,
            real_ms,
            persist_dir,
            ledger,
            script,
            allow_unregistered,
            fxp_mode,
            fxp_config,
        } => match build_fxp(&fxp_config, &fxp_mode) {
            Ok(Some((registry, config_bus))) => {
                // Barramento FXP real/híbrido/simulado configurado.
                let sim = script.build_simulator();
                let bus = FxpBus::build(registry, config_bus, sim);
                run(
                    &arquivo,
                    ticks,
                    real_ms,
                    persist_dir,
                    ledger,
                    script,
                    allow_unregistered,
                    bus,
                )
                .await
            }
            Ok(None) => {
                // Sem `--fxp-config`/`--fxp-mode`: simulador em processo,
                // paridade exata com a Etapa 2 (bit a bit).
                let sim = script.build_simulator();
                run(
                    &arquivo,
                    ticks,
                    real_ms,
                    persist_dir,
                    ledger,
                    script,
                    allow_unregistered,
                    sim,
                )
                .await
            }
            Err((code, msg)) => {
                eprintln!("{msg}");
                code
            }
        },
        Command::FxpProbe {
            fxp_mode,
            fxp_config,
        } => fxp_probe(&fxp_config, &fxp_mode),
        Command::LedgerVerify { arquivo } => ledger_verify(&arquivo),
    }
}

/// Resolve `--fxp-config`/`--fxp-mode` no backend do `run`:
/// - `Ok(None)` — flags ausentes → simulador puro da Etapa 2;
/// - `Ok(Some((registro, config)))` — barramento configurado;
/// - `Err((código, mensagem))` — erro de uso/config (mensagem pronta para
///   stderr, preservando os textos e códigos da interface original).
fn build_fxp(
    fxp_config: &Option<PathBuf>,
    fxp_mode: &Option<String>,
) -> Result<Option<(DeviceRegistry, BusConfig)>, (i32, String)> {
    if fxp_config.is_none() && fxp_mode.is_none() {
        return Ok(None);
    }
    let mut registry = DeviceRegistry::minimum();
    let mut cfg_fxp = None;
    if let Some(path) = fxp_config {
        let text = std::fs::read_to_string(path).map_err(|e| {
            (
                2,
                format!("vbl: não foi possível ler '{}': {e}", path.display()),
            )
        })?;
        let cfg = FxpConfig::parse(&text).map_err(|e| {
            (
                1,
                format!("vbl: config FXP inválida em '{}': {e}", path.display()),
            )
        })?;
        cfg.apply(&mut registry)
            .map_err(|e| (1, format!("vbl: registro FXP inválido: {e}")))?;
        cfg_fxp = Some(cfg);
    }
    // Modo: flag > arquivo de config > simulado (default).
    let mode = match fxp_mode.as_deref() {
        Some(m) => OperationMode::parse(m).map_err(|e| (2, format!("vbl: {e}")))?,
        None => cfg_fxp
            .as_ref()
            .and_then(|c| c.mode)
            .unwrap_or(OperationMode::Simulated),
    };
    let mut config = BusConfig {
        mode,
        ..Default::default()
    };
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
    Ok(Some((registry, config)))
}

// ----------------------------------------------------------------------
// vbl check
// ----------------------------------------------------------------------
fn check(arquivo: &str, with_registry: bool) -> i32 {
    let source = match std::fs::read_to_string(arquivo) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("vbl: não foi possível ler '{arquivo}': {e}");
            return 2;
        }
    };
    let (_program, diags) = vbl_lang::parse(&source);
    let mut diagnosticos = diags.items.clone();
    if with_registry && !diagnosticos.iter().any(|d| d.is_error()) {
        // validação contra o registro mínimo (FORMAL §3/§6)
        let (program, _) = vbl_lang::parse(&source);
        let fxp = FxpSimulator::new();
        for d in validate(fxp.registry(), &program) {
            diagnosticos.push(vbl_lang::Diagnostic::error(
                &d.code,
                vbl_lang::Span::default(),
                d.message,
            ));
        }
    }
    diagnosticos.sort_by_key(|d| (d.span.line, d.span.col));
    if diagnosticos.is_empty() {
        println!("ok: {arquivo} — programa válido");
        return 0;
    }
    for d in &diagnosticos {
        println!("{d}");
    }
    let errors = diagnosticos.iter().filter(|d| d.is_error()).count();
    eprintln!("{arquivo}: {errors} erro(s) de compilação");
    1
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
) -> i32 {
    let source = match std::fs::read_to_string(arquivo) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("vbl: não foi possível ler '{arquivo}': {e}");
            return 2;
        }
    };
    let (program, diags) = vbl_lang::parse(&source);
    let errors: Vec<&vbl_lang::Diagnostic> = diags.errors().collect();
    if !errors.is_empty() {
        for d in &diags.items {
            println!("{d}");
        }
        eprintln!(
            "vbl: {} erro(s) de compilação — programa não carregado",
            errors.len()
        );
        return 1;
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
            return 1;
        }
    }

    std::fs::create_dir_all(&persist_dir).expect("criar diretório de persistência");
    println!("▶ {arquivo} — relógio virtual 1 tick = 1s");

    match ledger {
        // Etapa 4 (PLAN §4.1): Caderno de produção — gravação assíncrona
        Some(binary) => {
            let production = match ProductionLedger::open(&binary) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("vbl: caderno '{}': {e}", binary.display());
                    return 2;
                }
            };
            println!(
                "  Caderno de produção: {} (assíncrono; JSONL em {})",
                binary.display(),
                jsonl_path(&binary).display()
            );
            let mut engine = Engine::with_ledger(fxp, 1.0, &persist_dir, production);
            reload(&mut engine);
            let mut interp = load(&mut engine, &program);
            println!("  {} forma(s) carregada(s)", engine.active_names().len());
            let interval = real_ms.map(|ms| tokio::time::interval(Duration::from_millis(ms)));
            let start = Instant::now();
            let executed = run_loop(
                &mut engine,
                &mut interp,
                ticks.unwrap_or(u64::MAX),
                interval,
                &script,
            )
            .await;
            let duration = start.elapsed();
            let ativos: Vec<(String, String, String)> = engine
                .active_names()
                .iter()
                .filter_map(|n| {
                    engine.form(n).map(|f| {
                        (
                            n.to_string(),
                            format!("{}", f.value),
                            f.conjugation.name().to_string(),
                        )
                    })
                })
                .collect();
            // consumo do Caderno encerra a thread de gravação (fechar)
            let summary = match engine.ledger.close() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("vbl: {e}");
                    return 1;
                }
            };
            run_summary(&ativos, executed, duration, Some(summary), Some(&binary))
        }
        // Sem --ledger: cadeia em memória (paridade com a Etapa 2)
        None => {
            let mut engine = Engine::new(fxp, 1.0, &persist_dir);
            reload(&mut engine);
            let mut interp = load(&mut engine, &program);
            println!("  {} forma(s) carregada(s)", engine.active_names().len());
            let interval = real_ms.map(|ms| tokio::time::interval(Duration::from_millis(ms)));
            let start = Instant::now();
            let executed = run_loop(
                &mut engine,
                &mut interp,
                ticks.unwrap_or(u64::MAX),
                interval,
                &script,
            )
            .await;
            let duration = start.elapsed();
            let ativos: Vec<(String, String, String)> = engine
                .active_names()
                .iter()
                .filter_map(|n| {
                    engine.form(n).map(|f| {
                        (
                            n.to_string(),
                            format!("{}", f.value),
                            f.conjugation.name().to_string(),
                        )
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
            0
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

/// Sumário comum dos dois caminhos de Caderno. Devolve 1 se a auditoria
/// externa reprovar (cadeia corrompida), 0 caso contrário.
fn run_summary(
    ativos: &[(String, String, String)],
    executed: u64,
    duration: Duration,
    summary: Option<vbl_runtime::production_ledger::Summary>,
    binary: Option<&Path>,
) -> i32 {
    println!(
        "■ {executed} tick(s) em {duration:.1?} — formas ativas restantes: {}",
        if ativos.is_empty() {
            "—".to_string()
        } else {
            const LIMIT: usize = 20;
            let resumo: Vec<String> = ativos
                .iter()
                .take(LIMIT)
                .map(|(n, v, c)| format!("{n}: {v} ({c})"))
                .collect();
            if ativos.len() > LIMIT {
                format!("{} … (+{} formas)", resumo.join(", "), ativos.len() - LIMIT)
            } else {
                resumo.join(", ")
            }
        }
    );
    let (Some(summary), Some(binary)) = (summary, binary) else {
        return 0;
    };
    println!(
        "  Caderno de produção: {} evento(s), {} bytes, {:.2} J acumulados (gravação assíncrona)",
        summary.events, summary.bytes, summary.total_joules
    );
    // verificação EXTERNA: relê o arquivo e recompõe a cadeia
    let rel = match verify(binary) {
        Ok(rel) => rel,
        Err(e) => {
            eprintln!("vbl: verificação do Caderno falhou: {e}");
            return 1;
        }
    };
    println!(
        "  cadeia SHA-256 {}: {} evento(s) no arquivo; atuações {}/{} ok; divergências (alertas): {}",
        if rel.chain_ok { "ÍNTEGRA" } else { "CORROMPIDA" },
        rel.events,
        rel.atuacoes_ok,
        rel.actuations,
        rel.alerts
    );
    println!(
        "  cabeça da cadeia: {}…",
        &rel.chain_head[..16.min(rel.chain_head.len())]
    );
    let jsonl = jsonl_path(binary);
    match jsonl_from_binary(binary, &jsonl) {
        Ok(n) => println!(
            "  log JSONL exportado para {} ({n} eventos)",
            jsonl.display()
        ),
        Err(e) => eprintln!("vbl: conversão JSONL falhou: {e}"),
    }
    if !rel.chain_ok {
        eprintln!("vbl: log do Caderno CORROMPIDO — execução não passou na auditoria");
        return 1;
    }
    0
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
fn ledger_verify(arquivo: &str) -> i32 {
    let path = Path::new(arquivo);
    let rel = match verify(path) {
        Ok(rel) => rel,
        Err(e) => {
            eprintln!("vbl: {e}");
            return 2;
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
            format!(
                "CORROMPIDA (primeiro evento inválido: {:?})",
                rel.first_broken
            )
        }
    );
    println!(
        "  eventos: {}; cabeça: {}…",
        rel.events,
        &rel.chain_head[..16.min(rel.chain_head.len())]
    );
    println!("  energia: {:.2} J acumulados", rel.total_joules);
    println!(
        "  atuações: {}/{} com sucesso; divergências (alertas): {}",
        rel.atuacoes_ok, rel.actuations, rel.alerts
    );
    let mut counts: Vec<_> = rel.counts.iter().collect();
    counts.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (kind, n) in &counts {
        println!("    {kind}: {n}");
    }
    if !rel.chain_ok {
        1
    } else {
        0
    }
}

// ----------------------------------------------------------------------
// vbl fxp-probe
// ----------------------------------------------------------------------
fn fxp_probe(fxp_config: &Option<PathBuf>, fxp_mode: &Option<String>) -> i32 {
    let (registry, config_bus) = match build_fxp(fxp_config, fxp_mode) {
        Ok(Some(t)) => t,
        Ok(None) => (DeviceRegistry::minimum(), BusConfig::default()),
        Err((code, msg)) => {
            eprintln!("{msg}");
            return code;
        }
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
    println!(
        "FXP — modo {mode_name} — {} dispositivo(s) no registro",
        devices.len()
    );
    println!(
        "{:<16} {:<26} {:<9} {:<34} disponibilidade",
        "dispositivo", "tipo", "unidade", "rota"
    );
    let mut sensor_ok = 0usize;
    let mut sensores = 0usize;
    for (name, kind, endpoint) in &devices {
        let (kind_label, unit) = match kind {
            DeviceKind::Sensor {
                quantity,
                unit,
                precision_pct,
                ..
            } => (
                format!("sensor {quantity} (±{precision_pct}%)"),
                unit.clone(),
            ),
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
        let route = bus
            .route_of(name)
            .map(|r| r.description())
            .unwrap_or_else(|| "—".into());
        let availability = match kind {
            DeviceKind::Sensor { .. } => {
                sensores += 1;
                let t0 = Instant::now();
                match vbl_runtime::fxp::Fxp::read_sensor(&mut bus, name, &mut ledger) {
                    Ok(v) => {
                        sensor_ok += 1;
                        format!("✓ {:.3} ({:?})", v, t0.elapsed())
                    }
                    Err(vbl_runtime::fxp::SensorFailure::Inaccessible) => {
                        "✗ inacessível (condição não avaliada — §4.7)".to_string()
                    }
                    Err(vbl_runtime::fxp::SensorFailure::NotRegistered) => {
                        "✗ não registrado".to_string()
                    }
                }
            }
            DeviceKind::Actor { .. } => actor_availability(endpoint),
        };
        println!(
            "{:<16} {:<26} {:<9} {:<34} {}",
            name, kind_label, unit, route, availability
        );
    }
    println!(
        "sensores: {sensor_ok}/{sensores} acessíveis; alertas registrados no Caderno desta sonda: {}",
        ledger.search("ALERT", &[]).len()
    );

    // Cobertura dos dispositivos obrigatórios (FORMAL §6) — falha de CI se
    // faltar algo no denominador canônico.
    let mandatory = [
        ("cpu_temp", "sensor"),
        ("cpu_power", "sensor"),
        ("attention", "sensor"),
        ("CpuPowerCap", "ator"),
        ("Fan", "ator"),
        ("StatusLed", "ator"),
    ];
    let missing: Vec<String> = mandatory
        .iter()
        .filter(|(n, _)| !bus.registry_rico().contains(n))
        .map(|(n, k)| format!("{n} ({k})"))
        .collect();
    if missing.is_empty() {
        println!(
            "cobertura obrigatória (§6): {}/{} ✓",
            mandatory.len(),
            mandatory.len()
        );
        0
    } else {
        println!(
            "cobertura obrigatória (§6): {}/{} — faltando: {}",
            mandatory.len() - missing.len(),
            mandatory.len(),
            missing.join(", ")
        );
        eprintln!("vbl: registro sem dispositivos obrigatórios (FORMAL §6)");
        1
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
            if dir.exists() {
                "✓ endpoint presente".into()
            } else {
                "✗ endpoint ausente".into()
            }
        }
        Endpoint::RaplConstraint { file }
        | Endpoint::HwmonPwm { file }
        | Endpoint::HwmonTemp { file } => {
            if file.exists() {
                "✓ endpoint presente".into()
            } else {
                "✗ endpoint ausente".into()
            }
        }
        Endpoint::Remote { addr } => match addr {
            RemoteAddr::Unix(p) => {
                if p.exists() {
                    "✓ socket presente".into()
                } else {
                    "✗ socket ausente".into()
                }
            }
            RemoteAddr::Tcp { host, port } => {
                match format!("{host}:{port}").parse::<std::net::SocketAddr>() {
                    Ok(alvo) => match std::net::TcpStream::connect_timeout(
                        &alvo,
                        Duration::from_millis(500),
                    ) {
                        Ok(_) => "✓ peer alcançável".into(),
                        Err(e) => format!("✗ conexão falhou ({e})"),
                    },
                    Err(_) => format!("✗ endereço inválido ({host}:{port})"),
                }
            }
        },
    }
}

// ----------------------------------------------------------------------
// Suíte in-process: cada subcomando ensaiado pelo `dispatch` (os testes E2E
// continuam cobrindo o binário fora de processo; aqui o foco é o caminho
// interno — códigos de saída, mensagens e efeitos em arquivo).
// ----------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAMA_OK: &str = "\
event Piscar {
    value: \"olho\",
    horizon: 5s
}
";
    /// `solar_panel` está fora do registro mínimo (FORMAL §6).
    const PROGRAMA_SENSOR_AUSENTE: &str = "\
event Vigia {
    value: 1,
    horizon: 5s,
    source_path: \"solar_panel\"
}
";
    const PROGRAMA_QUEBRADO: &str = "event SemCorpo {";

    fn roda(args: &[&str]) -> i32 {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(dispatch(args.iter().map(|s| s.to_string())))
    }

    fn tmp_dir(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vbl-cli-test-{}-{nome}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn grava(dir: &Path, nome: &str, conteudo: &str) -> PathBuf {
        let caminho = dir.join(nome);
        std::fs::write(&caminho, conteudo).unwrap();
        caminho
    }

    // ── jsonl_path ────────────────────────────────────────────────────────
    #[test]
    fn jsonl_path_acrescenta_sufixo() {
        assert_eq!(
            jsonl_path(Path::new("logs/a.vcad")),
            PathBuf::from("logs/a.vcad.jsonl")
        );
    }

    // ── build_fxp: as quatro resoluções de backend ────────────────────────
    #[test]
    fn build_fxp_sem_flags_da_simulador_puro() {
        assert!(build_fxp(&None, &None).unwrap().is_none());
    }

    #[test]
    fn build_fxp_modo_por_flag() {
        let (_, config) = build_fxp(&None, &Some("hibrido".into())).unwrap().unwrap();
        assert_eq!(config.mode, OperationMode::Hybrid);
        let (_, config) = build_fxp(&None, &Some("simulado".into())).unwrap().unwrap();
        assert_eq!(config.mode, OperationMode::Simulated);
    }

    #[test]
    fn build_fxp_config_aplica_tempos_e_modo() {
        let dir = tmp_dir("config-ok");
        let cfg = grava(
            &dir,
            "fxp.cfg",
            "\
mode = hibrido
cache_ttl_ms = 100
read_timeout_ms = 20
act_timeout_local_ms = 40
act_timeout_remote_ms = 400
queue_timeout_ms = 2500
retries = 3
",
        );
        let (registry, config) = build_fxp(&Some(cfg), &None).unwrap().unwrap();
        assert!(registry.contains("cpu_temp")); // mínimo preservado
        assert_eq!(config.mode, OperationMode::Hybrid); // modo veio da config
        assert_eq!(config.cache_ttl, Duration::from_millis(100));
        assert_eq!(config.read_timeout, Duration::from_millis(20));
        assert_eq!(config.act_timeout_local, Duration::from_millis(40));
        assert_eq!(config.act_timeout_remote, Duration::from_millis(400));
        assert_eq!(config.queue_timeout_ticks, 3); // 2500 ms → 3 ticks (teto)
        assert_eq!(config.retries, 3);
    }

    #[test]
    fn build_fxp_flag_sobrepoe_config_e_arredonda_prazo_da_fila() {
        let dir = tmp_dir("config-misto");
        let cfg = grava(&dir, "fxp.cfg", "mode = hibrido\nqueue_timeout_ms = 1\n");
        let (_, config) = build_fxp(&Some(cfg), &Some("real".into()))
            .unwrap()
            .unwrap();
        assert_eq!(config.mode, OperationMode::Real); // flag > config
        assert_eq!(config.queue_timeout_ticks, 1); // mínimo 1 tick
    }

    #[test]
    fn build_fxp_erros_de_uso_e_config_tem_codigo_e_mensagem() {
        // modo desconhecido → uso (2)
        let (code, msg) = build_fxp(&None, &Some("voador".into())).unwrap_err();
        assert_eq!(
            (code, msg.as_str()),
            (
                2,
                "vbl: config inválida: modo desconhecido: 'voador' (use real | simulado | hibrido)"
            )
        );
        // config ilegível → I/O (2)
        let (code, msg) =
            build_fxp(&Some(PathBuf::from("/nem-existe/vbl.cfg")), &None).unwrap_err();
        assert_eq!(code, 2);
        assert!(msg.starts_with("vbl: não foi possível ler '/nem-existe/vbl.cfg'"));
        // config malformada → config (1)
        let dir = tmp_dir("config-ruim");
        let cfg = grava(&dir, "fxp.cfg", "sem igual nesta linha\n");
        let (code, msg) = build_fxp(&Some(cfg), &None).unwrap_err();
        assert_eq!(code, 1);
        assert!(msg.contains("config FXP inválida"));
        // config válida que rejeita o registro → registro (1)
        let dir = tmp_dir("config-alias-quebrado");
        // alias_de é a chave real do parse; alvo inexistente falha no apply
        let cfg = grava(
            &dir,
            "fxp.cfg",
            "cpu_temp.alias_de = dispositivo_nem_existe\n",
        );
        let (code, msg) = build_fxp(&Some(cfg), &None).unwrap_err();
        assert_eq!(code, 1);
        assert!(msg.contains("registro FXP inválido"));
        assert!(msg.contains("aponta para dispositivo inexistente"));
    }

    // ── check ─────────────────────────────────────────────────────────────
    #[test]
    fn check_programa_valido() {
        let dir = tmp_dir("check-ok");
        let arq = grava(&dir, "ok.vl", PROGRAMA_OK);
        assert_eq!(roda(&["check", arq.to_str().unwrap()]), 0);
    }

    #[test]
    fn check_com_registro_pega_sensor_fora_do_minimo() {
        let dir = tmp_dir("check-registro");
        let arq = grava(&dir, "vigia.vl", PROGRAMA_SENSOR_AUSENTE);
        assert_eq!(roda(&["check", arq.to_str().unwrap()]), 1);
        // --no-registry: só o parser fala — programa carrega
        assert_eq!(roda(&["check", "--no-registry", arq.to_str().unwrap()]), 0);
    }

    #[test]
    fn check_programa_quebrado_e_arquivo_ausente() {
        let dir = tmp_dir("check-quebrado");
        let arq = grava(&dir, "quebrado.vl", PROGRAMA_QUEBRADO);
        assert_eq!(roda(&["check", arq.to_str().unwrap()]), 1);
        assert_eq!(
            roda(&["check", dir.join("nem-existe.vl").to_str().unwrap()]),
            2
        );
    }

    // ── run ───────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn run_sem_ledger_executa_e_soma_cadeia() {
        let dir = tmp_dir("run-memoria");
        let arq = grava(&dir, "ok.vl", PROGRAMA_OK);
        let persist = dir.join("persistence");
        let code = run(
            arq.to_str().unwrap(),
            Some(2),
            None,
            persist,
            None,
            Script::default(),
            false,
            FxpSimulator::new(),
        )
        .await;
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn run_com_ledger_de_producao_grava_vcad_e_jsonl() {
        let dir = tmp_dir("run-producao");
        let arq = grava(&dir, "ok.vl", PROGRAMA_OK);
        let ledger = dir.join("caderno.vcad");
        let code = run(
            arq.to_str().unwrap(),
            Some(3),
            None,
            dir.join("persistence"),
            Some(ledger.clone()),
            Script::default(),
            false,
            FxpSimulator::new(),
        )
        .await;
        assert_eq!(code, 0);
        assert!(ledger.is_file());
        assert!(jsonl_path(&ledger).is_file()); // export automático no sumário
                                                // o arquivo gravado passa na verificação EXTERNA
        assert!(verify(&ledger).unwrap().chain_ok);
    }

    #[tokio::test]
    async fn run_recusa_sensor_fora_do_registro_e_aceita_com_allow() {
        let dir = tmp_dir("run-registro");
        let arq = grava(&dir, "vigia.vl", PROGRAMA_SENSOR_AUSENTE);
        let comuns = [
            arq.to_str().unwrap().to_string(),
            "--ticks".to_string(),
            "2".to_string(),
            "--persist-dir".to_string(),
            dir.join("p").to_str().unwrap().to_string(),
        ];
        // sem flag: recusa (FORMAL §3/§6)
        let code = dispatch(std::iter::once("run".to_string()).chain(comuns.iter().cloned())).await;
        assert_eq!(code, 1);
        // com --allow-unregistered: executa com alertas (§4.7)
        let code = dispatch(
            std::iter::once("run".to_string())
                .chain(comuns.iter().cloned())
                .chain(std::iter::once("--allow-unregistered".to_string())),
        )
        .await;
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn run_erros_de_programa_e_de_io() {
        let dir = tmp_dir("run-erros");
        let quebrado = grava(&dir, "quebrado.vl", PROGRAMA_QUEBRADO);
        let code = run(
            quebrado.to_str().unwrap(),
            Some(1),
            None,
            dir.join("p"),
            None,
            Script::default(),
            false,
            FxpSimulator::new(),
        )
        .await;
        assert_eq!(code, 1);
        let code = run(
            dir.join("nem-existe.vl").to_str().unwrap(),
            None,
            None,
            dir.join("p"),
            None,
            Script::default(),
            false,
            FxpSimulator::new(),
        )
        .await;
        assert_eq!(code, 2);
    }

    // ── run_summary: auditoria reprova cadeia corrompida ─────────────────
    #[test]
    fn run_summary_reprova_caderno_corrompido() {
        let dir = tmp_dir("summary-corrompido");
        let ledger = dir.join("caderno.vcad");
        // um caderno real, de verdadeira execução:
        {
            use vbl_runtime::ledger::Ledger as _;
            let mut production = ProductionLedger::open(&ledger).unwrap();
            production.record("INFO", "evento de teste", Json::Obj(Default::default()));
            production.record("INFO", "segundo evento", Json::Obj(Default::default()));
            production.close().unwrap();
        }
        // adultera um byte no meio do arquivo (quebra a cadeia SHA-256)
        let mut data = std::fs::read(&ledger).unwrap();
        let meio = data.len() / 2;
        data[meio] ^= 0xFF;
        std::fs::write(&ledger, &data).unwrap();

        let summary = vbl_runtime::production_ledger::Summary {
            events: 2,
            bytes: data.len() as u64,
            chain_head: "0".repeat(64),
            total_joules: 0.0,
            joules_per_form: Default::default(),
            counts: Default::default(),
        };
        let code = run_summary(&[], 2, Duration::ZERO, Some(summary), Some(&ledger));
        assert_eq!(code, 1); // auditoria externa reprova
                             // e o caminho sem Caderno (memória) é sempre ok:
        assert_eq!(run_summary(&[], 2, Duration::ZERO, None, None), 0);
    }

    // ── ledger-verify ─────────────────────────────────────────────────────
    #[test]
    fn ledger_verify_fluxos_ok_corrompido_e_jsonl() {
        let dir = tmp_dir("verify");
        let arq = grava(&dir, "ok.vl", PROGRAMA_OK);
        let ledger = dir.join("caderno.vcad");
        assert_eq!(
            roda(&[
                "run",
                arq.to_str().unwrap(),
                "--ticks",
                "2",
                "--persist-dir",
                dir.join("p").to_str().unwrap(),
                "--ledger",
                ledger.to_str().unwrap()
            ]),
            0
        );

        // íntegro: 0
        assert_eq!(roda(&["ledger-verify", ledger.to_str().unwrap()]), 0);

        // JSONL exportado é reconhecido pelo formato (sem rodapé, extensão ≠ vcad)
        let jsonl = jsonl_path(&ledger);
        assert!(jsonl.is_file());
        assert_eq!(roda(&["ledger-verify", jsonl.to_str().unwrap()]), 0);

        // corrompido: 1, com o primeiro evento quebrado reportado
        let podre = dir.join("podre.vcad");
        std::fs::copy(&ledger, &podre).unwrap();
        let mut data = std::fs::read(&podre).unwrap();
        let meio = data.len() / 2;
        data[meio] ^= 0xFF;
        std::fs::write(&podre, &data).unwrap();
        assert_eq!(roda(&["ledger-verify", podre.to_str().unwrap()]), 1);

        // arquivo ausente: 2
        assert_eq!(
            roda(&[
                "ledger-verify",
                dir.join("nem-existe.vcad").to_str().unwrap()
            ]),
            2
        );
    }

    // ── fxp-probe ─────────────────────────────────────────────────────────
    #[test]
    fn fxp_probe_registro_minimo_cumpre_a_secao_6() {
        assert_eq!(roda(&["fxp-probe"]), 0);
    }

    #[test]
    fn fxp_probe_com_config_e_com_erro_de_config() {
        let dir = tmp_dir("probe");
        let cfg = grava(&dir, "fxp.cfg", "mode = simulado\ncache_ttl_ms = 50\n");
        assert_eq!(
            roda(&["fxp-probe", "--fxp-config", cfg.to_str().unwrap()]),
            0
        );
        // config ilegível → código 2 propagado pelo dispatch
        assert_eq!(roda(&["fxp-probe", "--fxp-config", "/nem-existe/v.cfg"]), 2);
    }

    // ── dispatch: uso e subcomando desconhecido ───────────────────────────
    #[tokio::test]
    async fn dispatch_erro_de_uso_devolve_dois() {
        assert_eq!(dispatch(std::iter::empty()).await, 2); // sem subcomando
        assert_eq!(dispatch(["ajudar"].iter().map(|s| s.to_string())).await, 2);
        assert_eq!(dispatch(["check"].iter().map(|s| s.to_string())).await, 2); // falta arquivo
    }

    // ── actor_availability: todos os tipos de endpoint ────────────────────
    #[test]
    fn disponibilidade_do_ator_por_tipo_de_endpoint() {
        assert_eq!(
            actor_availability(&Endpoint::Simulated),
            "✓ (sempre, simulado)"
        );
        assert_eq!(
            actor_availability(&Endpoint::Auto),
            "auto-descoberta no host (ver coluna rota)"
        );

        let dir = tmp_dir("endpoints");
        // endpoint de diretório presente × ausente
        let dir_ok = dir.join("thermal");
        std::fs::create_dir_all(&dir_ok).unwrap();
        assert_eq!(
            actor_availability(&Endpoint::ThermalZone {
                dir: dir_ok.clone()
            }),
            "✓ endpoint presente"
        );
        assert_eq!(
            actor_availability(&Endpoint::LedClass {
                dir: dir.join("led")
            }),
            "✗ endpoint ausente"
        );
        // endpoint de arquivo presente × ausente
        let file_ok = grava(&dir, "constraint", "0\n");
        assert_eq!(
            actor_availability(&Endpoint::RaplConstraint { file: file_ok }),
            "✓ endpoint presente"
        );
        assert_eq!(
            actor_availability(&Endpoint::HwmonPwm {
                file: dir.join("nem-existe")
            }),
            "✗ endpoint ausente"
        );
        // socket unix ausente
        let unix = Endpoint::Remote {
            addr: RemoteAddr::Unix(dir.join("fxpd.sock")),
        };
        assert_eq!(actor_availability(&unix), "✗ socket ausente");
        // TCP com endereço inválido (host não-IP não parseia como SocketAddr)
        let tcp_ruim = Endpoint::Remote {
            addr: RemoteAddr::Tcp {
                host: "host_invalido".to_string(),
                port: 1,
            },
        };
        assert!(actor_availability(&tcp_ruim).starts_with("✗ endereço inválido"));
        // TCP inalcançável: porta de descarte em endereço de loopback válido
        let tcp_morto = Endpoint::Remote {
            addr: RemoteAddr::Tcp {
                host: "127.0.0.1".to_string(),
                port: 1,
            },
        };
        assert!(actor_availability(&tcp_morto).starts_with("✗ conexão falhou"));
    }
}
