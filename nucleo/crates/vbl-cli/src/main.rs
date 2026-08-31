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
//! - `vbl caderno-verify ARQUIVO`: verificação EXTERNA do log do Caderno
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
//! - sem `--caderno`: cadeia SHA-256 em memória ([`ChainCaderno`], soma no
//!   final da execução);
//! - com `--caderno ARQUIVO`: Caderno de PRODUÇÃO — gravação assíncrona em
//!   buffer (thread dedicada), binário compacto `.vcad` em ARQUIVO e export
//!   JSONL em `ARQUIVO.jsonl`; a integridade é reavermelhada do arquivo ao
//!   final (agente externo: `vbl caderno-verify`).
//!
//! O loop assíncrono usa tokio (PLAN §2.2); o núcleo do engine é
//! determinístico (relógio virtual injetável) — a simulação roteirizada é
//! reproduzível tick a tick.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vbl_runtime::caderno::Caderno;
use vbl_runtime::caderno_producao::{jsonl_de_binario, verificar, CadernoProducao};
use vbl_runtime::json::Json;
use vbl_runtime::{carregar, validar, ChainCaderno, Engine, FxpSimulator, MainInterpreter};

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
        Comando::CadernoVerify { arquivo } => caderno_verify(&arquivo),
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
// vbl run (genérico no backend FXP e no Caderno: memória ou produção)
// ----------------------------------------------------------------------
#[allow(clippy::too_many_arguments)]
async fn run<F: vbl_runtime::fxp::Fxp>(
    arquivo: &str,
    ticks: Option<u64>,
    real_ms: Option<u64>,
    persist_dir: PathBuf,
    caderno: Option<PathBuf>,
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
    println!("▶ {arquivo} — relógio virtual 1 tick = 1s");

    match caderno {
        // Etapa 4 (PLAN §4.1): Caderno de produção — gravação assíncrona
        Some(binario) => {
            let producao = CadernoProducao::abrir(&binario).unwrap_or_else(|e| {
                eprintln!("vbl: caderno '{}': {e}", binario.display());
                std::process::exit(2);
            });
            println!("  Caderno de produção: {} (assíncrono; JSONL em {})",
                binario.display(),
                caminho_jsonl(&binario).display());
            let mut engine = Engine::com_caderno(fxp, 1.0, &persist_dir, producao);
            recarregar(&mut engine);
            let mut interp = carregar(&mut engine, &programa);
            println!("  {} forma(s) carregada(s)", engine.nomes_ativos().len());
            let intervalo = real_ms.map(|ms| tokio::time::interval(Duration::from_millis(ms)));
            let inicio = Instant::now();
            let executados = laco(&mut engine, &mut interp, ticks.unwrap_or(u64::MAX), intervalo, &roteiro).await;
            let duracao = inicio.elapsed();
            let ativos: Vec<(String, String, String)> = engine
                .nomes_ativos()
                .iter()
                .filter_map(|n| {
                    engine.forma(n).map(|f| {
                        (n.clone(), format!("{}", f.value), f.conjugation.nome().to_string())
                    })
                })
                .collect();
            // consumo do Caderno encerra a thread de gravação (fechar)
            let resumo = engine.caderno.fechar().unwrap_or_else(|e| {
                eprintln!("vbl: {e}");
                std::process::exit(1);
            });
            sumario_run(&ativos, executados, duracao, Some(resumo), Some(&binario));
        }
        // Sem --caderno: cadeia em memória (paridade com a Etapa 2)
        None => {
            let mut engine = Engine::novo(fxp, 1.0, &persist_dir);
            recarregar(&mut engine);
            let mut interp = carregar(&mut engine, &programa);
            println!("  {} forma(s) carregada(s)", engine.nomes_ativos().len());
            let intervalo = real_ms.map(|ms| tokio::time::interval(Duration::from_millis(ms)));
            let inicio = Instant::now();
            let executados = laco(&mut engine, &mut interp, ticks.unwrap_or(u64::MAX), intervalo, &roteiro).await;
            let duracao = inicio.elapsed();
            let ativos: Vec<(String, String, String)> = engine
                .nomes_ativos()
                .iter()
                .filter_map(|n| {
                    engine.forma(n).map(|f| {
                        (n.clone(), format!("{}", f.value), f.conjugation.nome().to_string())
                    })
                })
                .collect();
            sumario_run(&ativos, executados, duracao, None, None);
            // sumário da cadeia em memória (implementação de referência)
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
            println!(
                "  Caderno (memória): {eventos} evento(s), {vazamentos:.2} J acumulados; cadeia SHA-256 {}",
                if engine.caderno.verify_chain() { "ÍNTEGRA" } else { "CORROMPIDA" }
            );
            println!("  cabeça da cadeia: {}…", &engine.caderno.chain_head()[..16]);
        }
    }
}

/// Recarga das `equilibrium` persistidas (FORMAL §4.1).
fn recarregar<C: Caderno, F: vbl_runtime::fxp::Fxp>(engine: &mut Engine<F, C>) -> usize {
    let n = vbl_runtime::persist::recarregar_equilibrium(engine);
    if n > 0 {
        println!("↺ {n} equilibrium recarregada(s) do suporte estável");
    }
    n
}

/// O loop de ticks (relógio virtual; tempo real opcional).
async fn laco<C: Caderno, F: vbl_runtime::fxp::Fxp>(
    engine: &mut Engine<F, C>,
    interp: &mut MainInterpreter,
    total: u64,
    mut intervalo: Option<tokio::time::Interval>,
    roteiro: &Roteiro,
) -> u64 {
    let mut executados: u64 = 0;
    for _ in 0..total {
        if let Some(iv) = &mut intervalo {
            iv.tick().await; // modo tempo real (1 tick = período do intervalo)
        }
        interp.run_due(engine);
        engine.tick();
        executados += 1;
        if engine.nomes_ativos().is_empty() && roteiro.terminou(engine.clock) {
            break;
        }
    }
    executados
}

/// Sumário comum dos dois caminhos de Caderno.
fn sumario_run(
    ativos: &[(String, String, String)],
    executados: u64,
    duracao: Duration,
    resumo: Option<vbl_runtime::caderno_producao::Resumo>,
    binario: Option<&Path>,
) {
    println!(
        "■ {executados} tick(s) em {duracao:.1?} — formas ativas restantes: {}",
        if ativos.is_empty() {
            "—".to_string()
        } else {
            const LIMITE: usize = 20;
            let resumo: Vec<String> = ativos
                .iter()
                .take(LIMITE)
                .map(|(n, v, c)| format!("{n}: {v} ({c})"))
                .collect();
            if ativos.len() > LIMITE {
                format!("{} … (+{} formas)", resumo.join(", "), ativos.len() - LIMITE)
            } else {
                resumo.join(", ")
            }
        }
    );
    let (Some(resumo), Some(binario)) = (resumo, binario) else {
        return;
    };
    println!(
        "  Caderno de produção: {} evento(s), {} bytes, {:.2} J acumulados (gravação assíncrona)",
        resumo.eventos, resumo.bytes, resumo.joules_totais
    );
    // verificação EXTERNA: relê o arquivo e recompõe a cadeia
    let rel = verificar(binario).unwrap_or_else(|e| {
        eprintln!("vbl: verificação do Caderno falhou: {e}");
        std::process::exit(1);
    });
    println!(
        "  cadeia SHA-256 {}: {} evento(s) no arquivo; atuações {}/{} ok; divergências (alertas): {}",
        if rel.cadeia_ok { "ÍNTEGRA" } else { "CORROMPIDA" },
        rel.eventos,
        rel.atuacoes_ok,
        rel.atuacoes,
        rel.alertas
    );
    println!("  cabeça da cadeia: {}…", &rel.chain_head[..16.min(rel.chain_head.len())]);
    let jsonl = caminho_jsonl(binario);
    match jsonl_de_binario(binario, &jsonl) {
        Ok(n) => println!("  log JSONL exportado para {} ({n} eventos)", jsonl.display()),
        Err(e) => eprintln!("vbl: conversão JSONL falhou: {e}"),
    }
    if !rel.cadeia_ok {
        eprintln!("vbl: log do Caderno CORROMPIDO — execução não passou na auditoria");
        std::process::exit(1);
    }
}

/// Caminho do export JSONL associado ao binário do Caderno.
fn caminho_jsonl(binario: &Path) -> PathBuf {
    let mut caminho = binario.as_os_str().to_owned();
    caminho.push(".jsonl");
    PathBuf::from(caminho)
}

// ----------------------------------------------------------------------
// vbl caderno-verify — verificação externa (AGENTS §1.4)
// ----------------------------------------------------------------------
fn caderno_verify(arquivo: &str) {
    let caminho = Path::new(arquivo);
    let rel = match verificar(caminho) {
        Ok(rel) => rel,
        Err(e) => {
            eprintln!("vbl: {e}");
            std::process::exit(2);
        }
    };
    let formato = if rel.rodape_ok || caminho.extension().and_then(|e| e.to_str()) == Some("vcad") {
        "binário .vcad"
    } else {
        "JSONL"
    };
    println!("Caderno: {arquivo} ({formato})");
    println!(
        "  cadeia SHA-256: {}",
        if rel.cadeia_ok {
            "ÍNTEGRA".to_string()
        } else {
            format!("CORROMPIDA (primeiro evento inválido: {:?})", rel.primeiro_quebrado)
        }
    );
    println!("  eventos: {}; cabeça: {}…", rel.eventos, &rel.chain_head[..16.min(rel.chain_head.len())]);
    println!("  energia: {:.2} J acumulados", rel.joules_totais);
    println!("  atuações: {}/{} com sucesso; divergências (alertas): {}", rel.atuacoes_ok, rel.atuacoes, rel.alertas);
    let mut contagens: Vec<_> = rel.contagens.iter().collect();
    contagens.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (kind, n) in &contagens {
        println!("    {kind}: {n}");
    }
    if !rel.cadeia_ok {
        std::process::exit(1);
    }
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
