//! `vbl` — interpretador de console `.vl` (entregável da Etapa 2, PLAN §2.3).
//!
//! Subcomandos:
//! - `vbl check <arquivo.vl>`: valida o programa (parser + registro FXP
//!   mínimo) e imprime diagnósticos com linha/coluna;
//! - `vbl run <arquivo.vl>`: carrega o estado inicial na memória com FXP
//!   simulado e executa o loop de tick (relógio virtual por padrão; modo
//!   tempo real com `--real-ms`), com persistência `equilibrium` e Caderno
//!   auditável.
//!
//! O loop assíncrono usa tokio (PLAN §2.2); o núcleo do engine é
//! determinístico (relógio virtual injetável) — a simulação roteirizada é
//! reproduzível tick a tick.

use std::path::PathBuf;
use vbl_runtime::json::Json;
use vbl_runtime::{carregar, validar, ChainCaderno, Engine, FxpSimulator};

mod args;
mod roteiro;

use args::{parse_args, Comando};
use roteiro::Roteiro;

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
        Comando::Run { arquivo, ticks, real_ms, persist_dir, caderno, roteiro, permitir_sem_registro } => {
            run(&arquivo, ticks, real_ms, persist_dir, caderno, roteiro, permitir_sem_registro).await
        }
    }
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
// vbl run
// ----------------------------------------------------------------------
#[allow(clippy::too_many_arguments)]
async fn run(
    arquivo: &str,
    ticks: Option<u64>,
    real_ms: Option<u64>,
    persist_dir: PathBuf,
    caderno_path: Option<PathBuf>,
    roteiro: Roteiro,
    permitir_sem_registro: bool,
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

    // FXP simulado com registro mínimo + roteirização do cenário
    let fxp = roteiro.construir_simulador();
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

    let inicio = std::time::Instant::now();
    let mut intervalo = real_ms.map(|ms| tokio::time::interval(std::time::Duration::from_millis(ms)));
    let mut executados: u64 = 0;
    for _ in 0..total {
        if let Some(iv) = &mut intervalo {
            iv.tick().await; // modo tempo real (1 tick = período do intervalo)
        }
        roteiro.aplicar_antes_do_tick(engine.clock + 1, &mut engine.fxp);
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
