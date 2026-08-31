//! vbl-soak — execução longa com churn de formas (Etapa 5 — AGENTS §2.2:
//! "zero vazamentos de heap em longa execução (24h)"; PLAN §5.1).
//!
//! Ciclo: a cada tick entram `--vivas/3` formas `event` com horizonte de
//! 3 ticks e as vencidas dissolvem pelo caminho NATURAL do runtime
//! (scheduler → `dissolve_horizon`) — carga estacionária com renovação
//! contínua, o cenário que expõe "vazamento inerte" (estrutura em heap além
//! do horizon) e crescimento de fila/contadores.
//!
//! Relatório periódico (stdout): tick, formas ativas, prazos na fila, RSS
//! (VmRSS de /proc/self/status), retenção declarada. Ao final compara o RSS
//! do patamar com o final: crescimento além da tolerância com carga constante
//! ⇒ exit 1 (falha de soak). 24 h de parede: `--segundos 86400`.
//!
//! Uso:
//! ```text
//! vbl-soak [--vivas N] [--ticks T] [--segundos S] [--relatorio A_CADA]
//! ```

struct Config {
    vivas: usize,
    max_ticks: u64,
    max_segundos: u64,
    relatorio: u64,
}

fn args() -> Config {
    let mut c = Config { vivas: 10_000, max_ticks: u64::MAX, max_segundos: 0, relatorio: 10_000 };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut valor = |padrao: u64| -> u64 {
            it.next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(padrao)
        };
        match a.as_str() {
            "--vivas" => c.vivas = valor(c.vivas as u64) as usize,
            "--ticks" => c.max_ticks = valor(c.max_ticks),
            "--segundos" => c.max_segundos = valor(c.max_segundos),
            "--relatorio" => c.relatorio = valor(c.relatorio),
            outro => {
                eprintln!("vbl-soak: argumento desconhecido '{outro}'");
                eprintln!("uso: vbl-soak [--vivas N] [--ticks T] [--segundos S] [--relatorio A_CADA]");
                std::process::exit(2);
            }
        }
    }
    c
}

/// RSS do processo em bytes (VmRSS de /proc/self/status; 0 se indisponível).
fn rss_bytes() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else { return 0 };
    for linha in status.lines() {
        if let Some(resto) = linha.strip_prefix("VmRSS:") {
            let kb: u64 = resto
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
    let inicio = std::time::Instant::now();
    println!(
        "vbl-soak: vivas alvo = {}, teto = {} ticks / {} s de parede, relatório a cada {} ticks",
        cfg.vivas, cfg.max_ticks, cfg.max_segundos, cfg.relatorio
    );

    let dir = std::env::temp_dir().join(format!("vbl-soak-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // NoopCaderno: mede-se o RUNTIME (formas + escalonador). O logger de
    // referência em memória retém TODO evento por desenho (auditoria), e o
    // Caderno de produção já foi medido à parte (≲ 1 MB @ 10k — Etapa 4).
    let mut engine = vbl_runtime::Engine::com_caderno(
        vbl_runtime::FxpSimulator::novo(),
        1.0,
        &dir,
        vbl_runtime::caderno::NoopCaderno,
    );

    // carga estacionária: por_tick novas com horizonte 3 ticks → ~vivas vivas
    let por_tick = (cfg.vivas / 3).max(1);
    let mut n: u64 = 0;
    let mut tick: u64 = 0;
    let mut rss_patamar = 0u64;
    let mut rss_max = 0u64;

    while tick < cfg.max_ticks {
        if cfg.max_segundos > 0 && inicio.elapsed().as_secs() >= cfg.max_segundos {
            break;
        }
        for _ in 0..por_tick {
            engine.registrar_forma(forma_evento(&format!("c{n}"), engine.sim_time));
            n += 1;
        }
        engine.tick();
        tick += 1;

        let rss = rss_bytes();
        rss_max = rss_max.max(rss);
        // patamar: RSS após a carga estabilizar (3× horizonte de ticks)
        if tick == 10 {
            rss_patamar = rss;
            println!("# patamar inicial: tick 10, RSS = {} KiB", rss / 1024);
        }
        if tick.is_multiple_of(cfg.relatorio) {
            println!(
                "tick {:>9} | ativas {:>6} | prazos {:>6} | RSS {:>8} KiB | pico {:>8} KiB | {} s",
                tick,
                engine.formas_ativas().len(),
                engine.scheduler.len(),
                rss / 1024,
                rss_max / 1024,
                inicio.elapsed().as_secs(),
            );
        }
    }

    let rss_final = rss_bytes();
    let duracao = inicio.elapsed();
    println!(
        "# fim: {} ticks em {} s | ativas {} | prazos {} | RSS final {} KiB | pico {} KiB",
        tick,
        duracao.as_secs(),
        engine.formas_ativas().len(),
        engine.scheduler.len(),
        rss_final / 1024,
        rss_max / 1024,
    );

    // veredito: com carga CONSTANTE e renovação total das formas, o RSS final
    // não pode crescer além da tolerância sobre o patamar (10% + 4 MiB de
    // folga para fragmentação do alocador e caches do simulador de I/O)
    let tolerancia = rss_patamar + rss_patamar / 10 + 4 * 1024 * 1024;
    if rss_final > tolerancia {
        eprintln!(
            "SOAK FALHOU: RSS final {} KiB > patamar+tolerância {} KiB — crescimento não justificado",
            rss_final / 1024,
            tolerancia / 1024
        );
        std::process::exit(1);
    }
    println!("# SOAK OK: RSS estável (final ≤ patamar + 10% + 4 MiB)");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Forma `event` de carga com horizonte curto (renovação pelo runtime).
fn forma_evento(nome: &str, agora: f64) -> vbl_runtime::Form {
    use vbl_runtime::fxp::Value;
    use vbl_lang::Conjugation;
    vbl_runtime::Form {
        name: nome.into(),
        value: Value::Str(format!("carga-{nome}")),
        horizon_s: 3.0,
        creation_time: agora,
        conjugation: Conjugation::Event,
        currency: "CpuCycles".into(),
        source_path: None,
        classification: None,
        declared_maintenance_deadline: None,
        manutencao: None,
        exchange_mode: None,
        cost_bytes: None,
        rules: Vec::new(),
        dissolvida: false,
        horizon_versao: 0,
        manutencao_versao: 0,
    }
}
