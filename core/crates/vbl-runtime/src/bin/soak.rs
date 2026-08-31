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
    match parse_args(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("vbl-soak: {msg}");
            eprintln!("uso: vbl-soak [--alive-forms N] [--ticks T] [--seconds S] [--report A_CADA]");
            std::process::exit(2);
        }
    }
}

/// Parser puro dos argumentos (ensaio in-process; `args` só o amarra ao env).
/// Valor não numérico mantém o default (a carga é heurística, não contrato).
fn parse_args(it: impl Iterator<Item = String>) -> Result<Config, String> {
    let mut c = Config { alive: 10_000, max_ticks: u64::MAX, max_seconds: 0, report: 10_000 };
    let mut it = it.peekable();
    while let Some(a) = it.next() {
        let mut value = |default: u64| -> u64 {
            it.next().and_then(|v| v.parse().ok()).unwrap_or(default)
        };
        match a.as_str() {
            "--alive-forms" => c.alive = value(c.alive as u64) as usize,
            "--ticks" => c.max_ticks = value(c.max_ticks),
            "--seconds" => c.max_seconds = value(c.max_seconds),
            "--report" => c.report = value(c.report),
            other => return Err(format!("argumento desconhecido '{other}'")),
        }
    }
    Ok(c)
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
    std::process::exit(run(cfg));
}

/// O ciclo de soak com teto de ticks/segundos; devolve 0 (estável) ou
/// 1 (RSS cresceu além da tolerância) — sem process::exit no meio.
fn run(cfg: Config) -> i32 {
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
        return 1;
    }
    println!("# SOAK OK: RSS estável (final ≤ patamar + 10% + 4 MiB)");
    let _ = std::fs::remove_dir_all(&dir);
    0
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

// ── suíte in-process: parser de args, RSS e o ciclo enxuto (12 ticks) ─────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_defaults_overrides_e_erro() {
        // defaults (carga de referência: 10k vivas, teto infinito)
        let c = parse_args(std::iter::empty()).unwrap();
        assert_eq!((c.alive, c.max_ticks, c.max_seconds, c.report), (10_000, u64::MAX, 0, 10_000));
        // overrides completos
        let c = parse_args(["--alive-forms", "30", "--ticks", "12", "--seconds", "0",
                            "--report", "5"].iter().map(|s| s.to_string())).unwrap();
        assert_eq!((c.alive, c.max_ticks, c.max_seconds, c.report), (30, 12, 0, 5));
        // valor não numérico mantém o default (carga é heurística)
        let c = parse_args(["--ticks", "muito"].iter().map(|s| s.to_string())).unwrap();
        assert_eq!(c.max_ticks, u64::MAX);
        // argumento desconhecido → erro de uso (a main sai com 2)
        assert!(parse_args(["--voar"].iter().map(|s| s.to_string())).is_err());
    }

    #[test]
    fn rss_do_processo_e_forma_de_carga() {
        // em Linux com /proc, o RSS do próprio processo é positivo
        if std::path::Path::new("/proc/self/status").exists() {
            assert!(rss_bytes() > 0);
        }
        // forma de carga: event com horizonte de 3 ticks e renovável
        let f = event_form("c7", 42.0);
        assert_eq!(f.name, "c7");
        assert_eq!(f.horizon_s, 3.0);
        assert_eq!(f.creation_time, 42.0);
        assert!(!f.dissolved);
        assert!(f.rules.is_empty());
    }

    #[test]
    fn ciclo_enxuto_termina_estavel() {
        // 12 ticks com 3 vivas: cobre patamar (tick 10), relatórios periódicos
        // e o veredito final (RSS sob carga constante renova sem crescer)
        let cfg = Config { alive: 3, max_ticks: 12, max_seconds: 0, report: 5 };
        assert_eq!(run(cfg), 0);
    }
}
