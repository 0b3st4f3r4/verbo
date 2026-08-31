//! `vbl` — interpretador de console `.vl` (entregáveis das Etapas 2 e 3,
//! PLAN §2.3/§3).
//!
//! Subcomandos:
//! - `vbl check <arquivo.vl>`: valida o programa (parser + registro FXP
//!   mínimo) e imprime diagnósticos com linha/coluna;
//! - `vbl run <arquivo.vl>`: carrega o estado inicial na memória e executa o
//!   loop de tick (relógio virtual por padrão; modo tempo real com
//!   `--real-ms`), com persistência `equilibrium` e Caderno auditável;
//! - `vbl fxp-probe`: audita o registro FXP do host (dispositivo × modo ×
//!   rota × disponibilidade × latência) e a cobertura dos dispositivos
//!   obrigatórios (FORMAL §6).
//!
//! Backend FXP do `run` (PLAN Etapa 3):
//! - padrão: simulador determinístico em processo (paridade com a Etapa 2);
//! - `--fxp-config ARQUIVO [--fxp-mode MODO]`: barramento real (`FxpBus`)
//!   com registro rico, drivers reais (thermal_zone, RAPL, hwmon PWM, LED)
//!   e/ou peers remotos — dado sintético só circula em modo simulado/
//!   híbrido explícito, marcado no Caderno (FORMAL §4.7).
//!
//! O loop assíncrono usa tokio (PLAN §2.2); o núcleo do engine é
//! determinístico (relógio virtual injetável) — a simulação roteirizada é
//! reproduzível tick a tick.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use vbl_runtime::json::Json;
use vbl_runtime::{carregar, validar, ChainCaderno, Engine, FxpSimulator};

mod args;
mod roteiro;

use args::{parse_args, Comando};
use roteiro::Roteiro;
use vbl_fxp::registry::{DeviceKind, DeviceRegistry, Endpoint, FxpConfig, ModoOperacao, RemoteAddr};
use vbl_fxp::{BusConfig, FxpBus};

const REGISTRO_MINIMO: &str = "\
registro mínimo do FXP (FORMAL §6):
  sensores : cpu_temp (temperatura, °C), cpu_power (potencia, W), attention (atencao, %)
  atores   : CpuPowerCap [10..250, safety 200], Ventoinha [0..255, safety 200], LedIndicador
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
        Comando::Check { arquivo, com_registro } => check(&arquivo, com_registro),
        Comando::Run {
            arquivo, ticks, real_ms, persist_dir, caderno, roteiro, permitir_sem_registro,
            fxp_mode, fxp_config,
        } => match construir_fxp(&fxp_config, &fxp_mode) {
            Ok((registro, config_bus)) => {
                // Barramento FXP real/híbrido/simulado configurado.
                let sim = roteiro.construir_simulador();
                let bus = FxpBus::construir(registro, config_bus, sim);
                run(&arquivo, ticks, real_ms, persist_dir, caderno, roteiro, permitir_sem_registro, bus).await
            }
            Err(_) => {
                // Sem `--fxp-config`/`--fxp-mode`: simulador em processo,
                // paridade exata com a Etapa 2 (bit a bit).
                let sim = roteiro.construir_simulador();
                run(&arquivo, ticks, real_ms, persist_dir, caderno, roteiro, permitir_sem_registro, sim).await
            }
        },
        Comando::FxpProbe { fxp_mode, fxp_config } => fxp_probe(&fxp_config, &fxp_mode),
    }
}

/// Resolve `--fxp-config`/`--fxp-mode` em (registro rico, config do bus).
/// `Err(())` = flags ausentes → backend simulado puro da Etapa 2.
fn construir_fxp(
    fxp_config: &Option<PathBuf>,
    fxp_mode: &Option<String>,
) -> Result<(DeviceRegistry, BusConfig), ()> {
    if fxp_config.is_none() && fxp_mode.is_none() {
        return Err(());
    }
    let mut registro = DeviceRegistry::minimo();
    let mut cfg_fxp = None;
    if let Some(caminho) = fxp_config {
        let texto = std::fs::read_to_string(caminho).unwrap_or_else(|e| {
            eprintln!("vbl: não foi possível ler '{}': {e}", caminho.display());
            std::process::exit(2);
        });
        let cfg = FxpConfig::parse(&texto).unwrap_or_else(|e| {
            eprintln!("vbl: config FXP inválida em '{}': {e}", caminho.display());
            std::process::exit(1);
        });
        cfg.aplicar(&mut registro).unwrap_or_else(|e| {
            eprintln!("vbl: registro FXP inválido: {e}");
            std::process::exit(1);
        });
        cfg_fxp = Some(cfg);
    }
    // Modo: flag > arquivo de config > simulado (default).
    let modo = match fxp_mode.as_deref() {
        Some(m) => ModoOperacao::parse(m).unwrap_or_else(|e| {
            eprintln!("vbl: {e}");
            std::process::exit(2);
        }),
        None => cfg_fxp.as_ref().and_then(|c| c.mode).unwrap_or(ModoOperacao::Simulado),
    };
    let mut config = BusConfig { modo, ..Default::default() };
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
    Ok((registro, config))
}

// ----------------------------------------------------------------------
// vbl check
// ----------------------------------------------------------------------
fn check(arquivo: &str, com_registro: bool) {
    let fonte = match std::fs::read_to_string(arquivo) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("vbl: não foi possível ler '{arquivo}': {e}");
            std::process::exit(2);
        }
    };
    let (_programa, diags) = vbl_lang::parse(&fonte);
    let mut diagnosticos = diags.items.clone();
    if com_registro && !diagnosticos.iter().any(|d| d.is_error()) {
        // validação contra o registro mínimo (FORMAL §3/§6)
        let (programa, _) = vbl_lang::parse(&fonte);
        let fxp = FxpSimulator::novo();
        for d in validar(fxp.registry(), &programa) {
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
    let erros = diagnosticos.iter().filter(|d| d.is_error()).count();
    eprintln!("{arquivo}: {erros} erro(s) de compilação");
    std::process::exit(1);
}

// ----------------------------------------------------------------------
// vbl run (genérico no backend FXP: simulador da Etapa 2 ou FxpBus)
// ----------------------------------------------------------------------
#[allow(clippy::too_many_arguments)]
async fn run<F: vbl_runtime::fxp::Fxp>(
    arquivo: &str,
    ticks: Option<u64>,
    real_ms: Option<u64>,
    persist_dir: PathBuf,
    caderno_path: Option<PathBuf>,
    roteiro: Roteiro,
    permitir_sem_registro: bool,
    fxp: F,
) {
    let fonte = match std::fs::read_to_string(arquivo) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("vbl: não foi possível ler '{arquivo}': {e}");
            std::process::exit(2);
        }
    };
    let (programa, diags) = vbl_lang::parse(&fonte);
    let erros: Vec<&vbl_lang::Diagnostic> = diags.errors().collect();
    if !erros.is_empty() {
        for d in &diags.items {
            println!("{d}");
        }
        eprintln!("vbl: {} erro(s) de compilação — programa não carregado", erros.len());
        std::process::exit(1);
    }

    // Validação contra o registro do backend (FORMAL §3/§6).
    if !permitir_sem_registro {
        let diags_registro = validar(fxp.registry(), &programa);
        if !diags_registro.is_empty() {
            for d in &diags_registro {
                eprintln!("vbl: {d}");
            }
            eprintln!(
                "vbl: {} referência(ões) fora do registro do FXP — use --permitir-sem-registro para executar mesmo assim (falhas de I/O seguem FORMAL §4.7)",
                diags_registro.len()
            );
            eprintln!("{REGISTRO_MINIMO}");
            std::process::exit(1);
        }
    }

    std::fs::create_dir_all(&persist_dir).expect("criar diretório de persistência");
    let mut engine = Engine::novo(fxp, 1.0, &persist_dir);

    // inicialização: recarrega `equilibrium` persistidas (FORMAL §4.1)
    let recarregadas = vbl_runtime::persist::recarregar_equilibrium(&mut engine);
    if recarregadas > 0 {
        println!("↺ {recarregadas} equilibrium recarregada(s) do suporte estável");
    }

    let mut interp = carregar(&mut engine, &programa);
    let total = ticks.unwrap_or(u64::MAX);
    println!("▶ {arquivo} — {} forma(s) carregada(s); relógio virtual 1 tick = 1s", engine.nomes_ativos().len());

    let inicio = Instant::now();
    let mut intervalo = real_ms.map(|ms| tokio::time::interval(Duration::from_millis(ms)));
    let mut executados: u64 = 0;
    for _ in 0..total {
        if let Some(iv) = &mut intervalo {
            iv.tick().await; // modo tempo real (1 tick = período do intervalo)
        }
        interp.run_due(&mut engine);
        engine.tick();
        executados += 1;
        if engine.nomes_ativos().is_empty() && roteiro.terminou(engine.clock) {
            break;
        }
    }
    let duracao = inicio.elapsed();

    // sumário (sumário do runtime; Caderno integral exportado abaixo)
    println!("■ {} tick(s) em {:.1?} — formas ativas restantes: {}",
        executados, duracao,
        engine.nomes_ativos().iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", "));
    for nome in engine.nomes_ativos() {
        if let Some(f) = engine.forma(nome) {
            println!("  - {nome}: {} (conjugação {})", f.value, f.conjugation.nome());
        }
    }
    let eventos = engine.caderno.eventos.len();
    let vazamentos: f64 = engine
        .caderno
        .buscar("VAZAMENTO", &[])
        .iter()
        .filter_map(|e| match &e.extra {
            Json::Obj(c) => c.get("joules").and_then(|j| match j {
                Json::Num(n) => Some(*n),
                _ => None,
            }),
            _ => None,
        })
        .sum();
    println!("  Caderno: {eventos} evento(s), {vazamentos:.2} J acumulados; cadeia SHA-256 {}",
        if engine.caderno.verify_chain() { "ÍNTEGRA" } else { "CORROMPIDA" });
    println!("  cabeça da cadeia: {}…", &engine.caderno.chain_head()[..16]);

    if let Some(caminho) = caderno_path {
        let n = engine.caderno.export_jsonl(&caminho).expect("exportar caderno");
        println!("  log do Caderno exportado para {} ({n} eventos)", caminho.display());
    }
    let _ = ChainCaderno::HEAD_INICIAL; // (documentação da âncora da cadeia)
}

// ----------------------------------------------------------------------
// vbl fxp-probe
// ----------------------------------------------------------------------
fn fxp_probe(fxp_config: &Option<PathBuf>, fxp_mode: &Option<String>) {
    let (registro, config_bus) = match construir_fxp(fxp_config, fxp_mode) {
        Ok(t) => t,
        Err(()) => (DeviceRegistry::minimo(), BusConfig::default()),
    };
    let modo_nome = match config_bus.modo {
        ModoOperacao::Simulado => "simulado",
        ModoOperacao::Real => "real",
        ModoOperacao::Hibrido => "hibrido",
    };
    let mut bus = FxpBus::construir(registro, config_bus, FxpSimulator::novo());
    let mut caderno = ChainCaderno::new();

    // Dados próprios (o probe precisa de &mut bus para as leituras).
    let dispositivos: Vec<_> = bus
        .registry_rico()
        .dispositivos()
        .map(|d| (d.name.clone(), d.kind.clone(), d.endpoint.clone()))
        .collect();
    println!("FXP — modo {modo_nome} — {} dispositivo(s) no registro", dispositivos.len());
    println!("{:<16} {:<26} {:<9} {:<34} disponibilidade", "dispositivo", "tipo", "unidade", "rota");
    let mut sensor_ok = 0usize;
    let mut sensores = 0usize;
    for (nome, kind, endpoint) in &dispositivos {
        let (tipo, unidade) = match kind {
            DeviceKind::Sensor { grandeza, unidade, precisao_pct, .. } =>
                (format!("sensor {grandeza} (±{precisao_pct}%)"), unidade.clone()),
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
        let rota = bus.rota_de(nome).map(|r| r.descricao()).unwrap_or_else(|| "—".into());
        let disponibilidade = match kind {
            DeviceKind::Sensor { .. } => {
                sensores += 1;
                let t0 = Instant::now();
                match vbl_runtime::fxp::Fxp::read_sensor(&mut bus, nome, &mut caderno) {
                    Ok(v) => {
                        sensor_ok += 1;
                        format!("✓ {:.3} ({:?})", v, t0.elapsed())
                    }
                    Err(vbl_runtime::fxp::FalhaSensor::Inacessivel) =>
                        "✗ inacessível (condição não avaliada — §4.7)".to_string(),
                    Err(vbl_runtime::fxp::FalhaSensor::NaoRegistrado) => "✗ não registrado".to_string(),
                }
            }
            DeviceKind::Actor { .. } => disponibilidade_ator(endpoint),
        };
        println!("{:<16} {:<26} {:<9} {:<34} {}", nome, tipo, unidade, rota, disponibilidade);
    }
    println!(
        "sensores: {sensor_ok}/{sensores} acessíveis; alertas registrados no Caderno desta sonda: {}",
        caderno.buscar("ALERTA", &[]).len()
    );

    // Cobertura dos dispositivos obrigatórios (FORMAL §6) — falha de CI se
    // faltar algo no denominador canônico.
    let obrigatorios = [
        ("cpu_temp", "sensor"), ("cpu_power", "sensor"), ("attention", "sensor"),
        ("CpuPowerCap", "ator"), ("Ventoinha", "ator"), ("LedIndicador", "ator"),
    ];
    let faltando: Vec<String> = obrigatorios
        .iter()
        .filter(|(n, _)| !bus.registry_rico().contains(n))
        .map(|(n, k)| format!("{n} ({k})"))
        .collect();
    if faltando.is_empty() {
        println!("cobertura obrigatória (§6): {}/{} ✓", obrigatorios.len(), obrigatorios.len());
    } else {
        println!("cobertura obrigatória (§6): {}/{} — faltando: {}",
            obrigatorios.len() - faltando.len(), obrigatorios.len(),
            faltando.join(", "));
        eprintln!("vbl: registro sem dispositivos obrigatórios (FORMAL §6)");
        std::process::exit(1);
    }
}

/// Disponibilidade de ator SEM atuar (probe é somente leitura): rota simulada
/// é sempre disponível; rota real confere a existência do endpoint; rota
/// remota confere o socket; inacessível reporta o motivo.
fn disponibilidade_ator(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Simulado => "✓ (sempre, simulado)".to_string(),
        Endpoint::Auto => "auto-descoberta no host (ver coluna rota)".to_string(),
        Endpoint::ThermalZone { dir }
        | Endpoint::RaplEnergy { dir }
        | Endpoint::LedClass { dir } => {
            if dir.exists() { "✓ endpoint presente".into() } else { "✗ endpoint ausente".into() }
        }
        Endpoint::RaplConstraint { file } | Endpoint::HwmonPwm { file } => {
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
