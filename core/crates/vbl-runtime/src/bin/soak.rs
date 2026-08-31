//! vbl-soak — execução longa com churn de formas (Etapa 5 — AGENTS §2.2:
//! "zero vazamentos de heap em longa execução (24h)"; PLAN §5.1).
//!
//! Ciclo: a cada tick entram `--alive/3` formas `event` com horizonte de
//! 3 ticks e as vencidas dissolvem pelo caminho NATURAL do runtime
//! (scheduler → `dissolve_horizon`) — carga estacionária com renovação
//! contínua, o cenário que expõe "vazamento inerte" (estrutura em heap além
//! do horizon) e crescimento de fila/contadores.
//!
//! Relatório periódico (stdout): tick, formas ativas, prazos na fila, RSS
//! (VmRSS de /proc/self/status), retenção declarada. Ao final compara o RSS
//! do patamar com o final: crescimento além da tolerância com carga constante
//! ⇒ exit 1 (falha de soak). 24 h de parede: `--seconds 86400`.
//!
//! Uso:
//! ```text
//! vbl-soak [--alive-forms N] [--ticks T] [--seconds S] [--report A_CADA]
//! ```

struct Config {
    alive: usize,
    max_ticks: u64,
    max_seconds: u64,
    report: u64,
}

fn args() -> Config {
    let mut c = Config { alive: 10_000, max_ticks: u64::MAX, max_seconds: 0, report: 10_000 };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = |default: u64| -> u64 {
            it.next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        match a.as_str() {
            "--alive-forms" => c.alive = value(c.alive as u64) as usize,
            "--ticks" => c.max_ticks = value(c.max_ticks),
            "--seconds" => c.max_seconds = value(c.max_seconds),
            "--report" => c.report = value(c.report),
            other => {
                eprintln!("vbl-soak: argumento desconhecido '{other}'");
                eprintln!("uso: vbl-soak [--alive-forms N] [--ticks T] [--seconds S] [--report A_CADA]");
                std::process::exit(2);
            }
        }
    }
    c
}

/// RSS do processo em bytes (VmRSS de /proc/self/status; 0 se indisponível).
fn rss_bytes() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else { return 0 };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            return kb * 1024;
        }
    }
    0
}

fn main() {
    let cfg = args();
    let start = std::time::Instant::now();
    println!(
        "vbl-soak: vivas alvo = {}, teto = {} ticks / {} s de parede, relatório a cada {} ticks",
        cfg.alive, cfg.max_ticks, cfg.max_seconds, cfg.report
    );

    let dir = std::env::temp_dir().join(format!("vbl-soak-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // NoopCaderno: mede-se o RUNTIME (formas + escalonador). O logger de
    // referência em memória retém TODO evento por desenho (auditoria), e o
    // Caderno de produção já foi medido à parte (≲ 1 MB @ 10k — Etapa 4).
    let mut engine = vbl_runtime::Engine::with_ledger(
        vbl_runtime::FxpSimulator::new(),
        1.0,
        &dir,
        vbl_runtime::ledger::NoopLedger,
    );

    // carga estacionária: per_tick novas com horizonte 3 ticks → ~vivas vivas
    let per_tick = (cfg.alive / 3).max(1);
    let mut n: u64 = 0;
    let mut tick: u64 = 0;
    let mut rss_threshold = 0u64;
    let mut rss_max = 0u64;

    while tick < cfg.max_ticks {
        if cfg.max_seconds > 0 && start.elapsed().as_secs() >= cfg.max_seconds {
            break;
        }
        for _ in 0..per_tick {
            engine.register_form(event_form(&format!("c{n}"), engine.sim_time));
            n += 1;
        }
        engine.tick();
        tick += 1;

        let rss = rss_bytes();
        rss_max = rss_max.max(rss);
        // patamar: RSS após a carga estabilizar (3× horizonte de ticks)
        if tick == 10 {
            rss_threshold = rss;
            println!("# patamar inicial: tick 10, RSS = {} KiB", rss / 1024);
        }
        if tick.is_multiple_of(cfg.report) {
            println!(
                "tick {:>9} | ativas {:>6} | prazos {:>6} | RSS {:>8} KiB | pico {:>8} KiB | {} s",
                tick,
                engine.active_forms().len(),
                engine.scheduler.len(),
                rss / 1024,
                rss_max / 1024,
                start.elapsed().as_secs(),
            );
        }
    }

    let rss_final = rss_bytes();
    let duration = start.elapsed();
    println!(
        "# fim: {} ticks em {} s | ativas {} | prazos {} | RSS final {} KiB | pico {} KiB",
        tick,
        duration.as_secs(),
        engine.active_forms().len(),
        engine.scheduler.len(),
        rss_final / 1024,
        rss_max / 1024,
    );

    // veredito: com carga CONSTANTE e renovação total das formas, o RSS final
    // não pode crescer além da tolerância sobre o patamar (10% + 4 MiB de
    // folga para fragmentação do alocador e caches do simulador de I/O)
    let tolerance = rss_threshold + rss_threshold / 10 + 4 * 1024 * 1024;
    if rss_final > tolerance {
        eprintln!(
            "SOAK FALHOU: RSS final {} KiB > patamar+tolerância {} KiB — crescimento não justificado",
            rss_final / 1024,
            tolerance / 1024
        );
        std::process::exit(1);
    }
    println!("# SOAK OK: RSS estável (final ≤ patamar + 10% + 4 MiB)");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Forma `event` de carga com horizonte curto (renovação pelo runtime).
fn event_form(name: &str, now: f64) -> vbl_runtime::Form {
    use vbl_runtime::fxp::Value;
    use vbl_lang::Conjugation;
    vbl_runtime::Form {
        name: name.into(),
        value: Value::Str(format!("carga-{name}")),
        horizon_s: 3.0,
        creation_time: now,
        conjugation: Conjugation::Event,
        currency: "CpuCycles".into(),
        source_path: None,
        classification: None,
        declared_maintenance_deadline: None,
        maintenance: None,
        exchange_mode: None,
        cost_bytes: None,
        rules: Vec::new(),
        dissolved: false,
        horizon_version: 0,
        maintenance_version: 0,
    }
}
