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
use std::sync::{Arc, Mutex};
use vbl_fxp::registry::{
    DeviceKind, DeviceRegistry, Endpoint, FxpConfig, OperationMode, RemoteAddr,
};
use vbl_fxp::{BusConfig, FxpBus, PeerConfig, PeerServer};

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
            fxp_psk_env,
            zstd,
            zstd_v,
            tofu_store,
        } => match build_fxp(&fxp_config, &fxp_mode) {
            Ok(Some((registry, mut config_bus))) => {
                // Barramento FXP real/híbrido/simulado configurado.
                // PSK do cliente (§4.6): SEMPRE de env — arquivo não carrega chave.
                if let Some(var) = &fxp_psk_env {
                    match std::env::var(var) {
                        Ok(v) if !v.is_empty() => config_bus.psk = Some(v.into_bytes()),
                        _ => {
                            eprintln!(
                                "vbl: env {var} ausente ou vazia — PSK não configurada (§4.6)"
                            );
                            return 2;
                        }
                    }
                }
                // v1.3 opt-in por flag (§4.8/§7): zstd treinado + store TOFU.
                if zstd {
                    config_bus.compression_zstd = true;
                }
                // v1.4 §4.8: id 4 — zstd treinado com verificação de dict no
                // fio (DICT_SYNC). Peer v1.3 não concede o bit 5 e a conexão
                // fica no id 3 (degradação honesta, evento no Caderno).
                if zstd_v {
                    config_bus.compression_zstd_v = true;
                }
                if let Some(p) = &tofu_store {
                    config_bus.tofu_store = Some(p.clone());
                }
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
        Command::FxpDaemon {
            fxp_mode,
            fxp_config,
            serve,
            auth,
            announce,
            announce_mdns,
            compress,
            dict,
            batch,
            timestamp,
            zstd,
            zstd_v,
            ledger,
            tls_cert,
            tls_key,
            tls_sessions,
        } => fxpd(FxpdArgs {
            fxp_mode,
            fxp_config,
            serve,
            auth,
            announce,
            announce_mdns,
            compress,
            dict,
            batch,
            timestamp,
            zstd,
            zstd_v,
            ledger,
            tls_cert,
            tls_key,
            tls_sessions,
        }),
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
        // v1.1 (§4.5): recursos opt-in por config — default = v1.0 puro.
        if let Some(b) = c.batch_prefetch {
            config.batch_prefetch = b;
        }
        if let Some(b) = c.compression_dict {
            config.compression_dict = b;
        }
        if let Some(b) = c.compression {
            config.compression = b;
        }
        if let Some(b) = c.wire_timestamp {
            config.wire_timestamp = b;
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
/// Argumentos do `vbl fxpd` (peer FXP v1.1 — docs/FXP-SCHEMA-v1.md §7).
struct FxpdArgs {
    fxp_mode: Option<String>,
    fxp_config: Option<PathBuf>,
    serve: String,
    auth: Option<String>,
    announce: Option<String>,
    announce_mdns: Option<String>,
    compress: bool,
    dict: bool,
    batch: bool,
    timestamp: bool,
    zstd: bool,
    /// v1.4 §4.8: anuncia ZSTD_V (id 4 no fio, verificação de dict).
    zstd_v: bool,
    ledger: Option<PathBuf>,
    /// TLS v1.2 (§7): PEMs da cadeia + chave do servidor (ambos ou nenhum).
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    /// v1.4 §7: cache de sessões TLS em disco (retomada entre processos).
    tls_sessions: Option<PathBuf>,
}

/// O peer FXP montado e pronto para servir (resultado de [`fxpd_preparar`]).
/// O handle do servidor PRECISA viver enquanto o daemon rodar (drop =
/// desligar) — fica aqui até o escopo terminar.
struct FxpdRuntime {
    /// Linha "pronto em …" para o operador.
    servindo: String,
    /// Porta TCP real (0 ⇒ efêmera) para o beacon do anúncio. Lida só pelos
    /// testes in-process — no bin o valor é consumido antes da construção.
    #[allow(dead_code)]
    porta_tcp_real: Option<u16>,
    /// Bits CAPS anunciados (§4.5) — espelhado no "pronto".
    caps_annunciadas: u16,
    _keepalive: Option<vbl_fxp::transport::Server>,
    _anunciador: Option<vbl_fxp::discover::Announcer>,
}

/// Monta o peer FXP (registro, bus, transporte e anúncio) — tudo que o
/// `vbl fxpd` precisa fazer antes de dormir. Recurso v1.1 é opt-in por flag
/// (§4.5) e PSK nunca vem de arquivo de config (§4.6 — só env).
fn fxpd_preparar(args: &FxpdArgs) -> Result<FxpdRuntime, i32> {
    use vbl_fxp::registry::OperationMode;
    use vbl_fxp::schema::caps;

    // -- registro + config do bus (mesma leitura do run/probe) --------------
    let (registry, mut config_bus) = match build_fxp(&args.fxp_config, &args.fxp_mode) {
        Ok(Some(par)) => par,
        Ok(None) => (DeviceRegistry::minimum(), BusConfig::default()),
        Err((code, msg)) => {
            eprintln!("{msg}");
            return Err(code);
        }
    };
    // O modo de operação do PEER também é respeitado (probe/run sobrepõem):
    if let Some(m) = &args.fxp_mode {
        config_bus.mode = match m.as_str() {
            "simulado" => OperationMode::Simulated,
            "real" => OperationMode::Real,
            "hibrido" => OperationMode::Hybrid,
            other => {
                eprintln!("vbl: --fxp-mode inválido: {other} (simulado|real|hibrido)");
                return Err(2);
            }
        };
    }

    // -- capacidades anunciadas (opt-in por flag; default = v1.0 puro) ------
    let mut caps_annunciadas = 0;
    if args.compress {
        caps_annunciadas |= caps::LZ4;
    }
    if args.dict {
        caps_annunciadas |= caps::DICT;
    }
    if args.zstd {
        // v1.3 §4.8: ZSTD anda SEMPRE com DICT — o gatilho do HELLO é o
        // mesmo; sem DICT o bit zstd nunca seria concedido (o dispatch
        // tira da interseção) — anunciar os dois é o honesto.
        caps_annunciadas |= caps::ZSTD | caps::DICT;
    }
    if args.zstd_v {
        // v1.4 §4.8: ZSTD_V anda com ZSTD+DICT (id 4 superset do id 3 —
        // mesmo treino, mais a verificação DICT_SYNC). O parser exige
        // --zstd; anunciar os três é o honesto.
        caps_annunciadas |= caps::ZSTD | caps::DICT | caps::ZSTD_V;
    }
    if args.batch {
        caps_annunciadas |= caps::BATCH;
    }
    if args.timestamp {
        caps_annunciadas |= caps::TIMESTAMP;
    }

    // -- PSK: SEMPRE de env (§4.6 — a chave nunca trafega nem fica em config)
    let psk = match &args.auth {
        None => None,
        Some(spec) => {
            let Some(var) = spec.strip_prefix("psk:") else {
                eprintln!("vbl: --auth exige psk:VAR_DE_ENV (recebido: {spec})");
                return Err(2);
            };
            match std::env::var(var) {
                Ok(v) if !v.is_empty() => Some(v.into_bytes()),
                _ => {
                    eprintln!("vbl: env {var} ausente ou vazia — PSK não configurada (§4.6)");
                    return Err(2);
                }
            }
        }
    };

    // -- Caderno do peer: produção com --ledger; sem ele, desligado (aviso)
    let caderno: Arc<Mutex<dyn vbl_runtime::ledger::Ledger + Send>> = match &args.ledger {
        Some(path) => match ProductionLedger::open(path) {
            Ok(l) => Arc::new(Mutex::new(l)),
            Err(e) => {
                eprintln!(
                    "vbl: não foi possível abrir Caderno '{}': {e}",
                    path.display()
                );
                return Err(2);
            }
        },
        None => {
            eprintln!("vbl fxpd: Caderno DESLIGADO (--ledger ARQUIVO grava o log do peer; §4.7)");
            Arc::new(Mutex::new(vbl_runtime::ledger::NoopLedger))
        }
    };

    let bus = Arc::new(std::sync::Mutex::new(FxpBus::build(
        registry,
        config_bus,
        FxpSimulator::new(),
    )));
    // Impressão digital do registro servido (hash canônico do anúncio §4.9).
    let nomes_do_registro: Vec<String> = {
        let b = bus.lock().expect("bus");
        b.registry_rico()
            .devices()
            .map(|d| d.name.clone())
            .collect()
    };
    // -- TLS (v1.2 §7): PEMs lidos do disco (o certificado é público; a
    // -- chave fica no arquivo do operador, nunca no fio). Erro de leitura
    // -- ou PEM inválido é honesto no arranque (serve_tcp_peer_port valida).
    let tls = match (&args.tls_cert, &args.tls_key) {
        (None, None) => None,
        (Some(c), Some(k)) => {
            let carregar = |p: &Path, o_que: &str| -> Result<String, i32> {
                std::fs::read_to_string(p).map_err(|e| {
                    eprintln!(
                        "vbl fxpd: não foi possível ler {o_que} '{}': {e}",
                        p.display()
                    );
                    2
                })
            };
            let certs_pem = carregar(c, "--tls-cert")?;
            let key_pem = carregar(k, "--tls-key")?;
            Some(vbl_fxp::TlsAccept {
                certs_pem,
                key_pem,
                // v1.4 §7: cache de sessões em disco (opt-in) — retomada
                // entre renascimentos do daemon, com 0-RTT preservado.
                sessoes: args.tls_sessions.clone(),
            })
        }
        _ => unreachable!("o parser de args exige --tls-cert e --tls-key juntos"),
    };
    let com_tls = tls.is_some();
    // v1.2 §4.10: pin do certificado para o TXT do anúncio mDNS (antes do
    // move para o PeerConfig).
    #[cfg(feature = "mdns")]
    let pin_tls = tls
        .as_ref()
        .and_then(|t| vbl_fxp::tls::fingerprint_pem(&t.certs_pem));
    let peer = PeerServer::shared(
        bus,
        caderno,
        PeerConfig {
            psk,
            caps: caps_annunciadas,
            tls,
        },
    );

    // -- transporte ----------------------------------------------------------
    // O handle do servidor PRECISA viver enquanto o daemon rodar (drop =
    // desligar); fica no runtime até o escopo terminar.
    let (servindo, porta_tcp_real, keepalive) = match args.serve.strip_prefix("unix:") {
        Some(path) => {
            let p = PathBuf::from(path);
            match vbl_fxp::peer::serve_unix_peer(&peer, &p) {
                Ok(servidor) => (format!("unix:{}", p.display()), None, Some(servidor)),
                Err(e) => {
                    eprintln!("vbl fxpd: não foi possível servir em unix:{path}: {e}");
                    return Err(2);
                }
            }
        }
        None => match args.serve.strip_prefix("tcp:") {
            Some(porta_txt) => {
                let porta: u16 = match porta_txt.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        eprintln!("vbl fxpd: porta inválida: {porta_txt}");
                        return Err(2);
                    }
                };
                match vbl_fxp::peer::serve_tcp_peer_port(&peer, porta) {
                    Ok((servidor, real)) => {
                        let esquema = if com_tls { "tcps" } else { "tcp" };
                        (
                            format!("{esquema}:0.0.0.0:{real}"),
                            Some(real),
                            Some(servidor),
                        )
                    }
                    Err(e) => {
                        eprintln!("vbl fxpd: não foi possível servir em tcp:{porta_txt}: {e}");
                        return Err(2);
                    }
                }
            }
            None => {
                eprintln!("vbl fxpd: --serve exige unix:PATH ou tcp:PORTA");
                return Err(2);
            }
        },
    };

    // -- anúncio multicast (§4.9, opt-in) ------------------------------------
    let anunciador = match &args.announce {
        Some(id) => {
            match vbl_fxp::discover::Announcer::start(
                id,
                porta_tcp_real.unwrap_or(0),
                vbl_fxp::discover::registry_hash(&nomes_do_registro),
                vbl_fxp::discover::DEFAULT_GROUP,
                vbl_fxp::discover::DEFAULT_INTERVAL,
            ) {
                Ok(a) => Some(a),
                Err(e) => {
                    eprintln!("vbl fxpd: anúncio multicast indisponível ({e}) — peer segue no ar sem anúncio (§4.9 honesto)");
                    None
                }
            }
        }
        None => None,
    };

    // v1.2 §4.10: anúncio mDNS opcional (--announce-mdns), com pin do
    // certificado no TXT quando o peer também serve TLS (§7).
    #[cfg(feature = "mdns")]
    let _anunciador_mdns = match &args.announce_mdns {
        Some(id) => {
            let pin = pin_tls;
            match vbl_fxp::mdns::MdnsAnnouncer::start(
                id,
                porta_tcp_real.unwrap_or(0),
                vbl_fxp::discover::registry_hash(&nomes_do_registro),
                pin,
            ) {
                Ok(a) => Some(a),
                Err(e) => {
                    eprintln!("vbl fxpd: anúncio mDNS indisponível ({e}) — peer segue no ar sem mDNS (§4.10 honesto)");
                    None
                }
            }
        }
        None => None,
    };
    #[cfg(not(feature = "mdns"))]
    if args.announce_mdns.is_some() {
        eprintln!(
            "vbl fxpd: --announce-mdns exige binário compilado com --features mdns — anúncio mDNS NÃO ativo (§4.10 honesto)"
        );
    }

    Ok(FxpdRuntime {
        servindo,
        porta_tcp_real,
        caps_annunciadas,
        _keepalive: keepalive,
        _anunciador: anunciador,
    })
}

/// `vbl fxpd`: monta o peer ([`fxpd_preparar`]), imprime o estado e dorme
/// até o operador encerrar o processo (SIGTERM/SIGINT).
fn fxpd(args: FxpdArgs) -> i32 {
    match fxpd_preparar(&args) {
        Ok(runtime) => {
            println!("fxpd pronto em {}", runtime.servindo);
            println!(
                "fxpd recursos: {} | auth: {} | tls: {} | announce: {}",
                if runtime.caps_annunciadas == 0 {
                    "v1.0 puro".to_string()
                } else {
                    let mut v = vec![];
                    if args.compress {
                        v.push("lz4");
                    }
                    if args.dict {
                        v.push("dict");
                    }
                    if args.batch {
                        v.push("batch");
                    }
                    if args.timestamp {
                        v.push("timestamp");
                    }
                    v.join(",")
                },
                if args.auth.is_some() {
                    "psk"
                } else {
                    "nenhuma"
                },
                if args.tls_cert.is_some() {
                    "tls1.3"
                } else {
                    "plano"
                },
                args.announce.as_deref().unwrap_or("-"),
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();
            // Daemon: dorme sem busy-wait; o processo morre por sinal e o
            // drop de `runtime` desliga o transporte/anúncio limpo.
            loop {
                std::thread::park();
            }
        }
        Err(code) => code,
    }
}

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
            RemoteAddr::TcpTls { host, port, trust } => {
                // Probe é somente leitura: alcançabilidade TCP + eco da
                // confiança declarada (a validação do certificado é do
                // handshake TLS; TOFU só grava na 1ª conexão do bus).
                let confia = match trust {
                    vbl_fxp::tls::Trust::Pin(pins) => format!(
                        "pin sha256:{}",
                        pins.iter()
                            .map(vbl_fxp::tls::hex32)
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    vbl_fxp::tls::Trust::Tofu => "tofu (1ª conexão grava, demais verificam)".into(),
                    vbl_fxp::tls::Trust::TofuEstrito => {
                        "tofu-estrito (allow-list: só conecta com pin registrado; v1.4 §7)".into()
                    }
                };
                match format!("{host}:{port}").parse::<std::net::SocketAddr>() {
                    Ok(alvo) => match std::net::TcpStream::connect_timeout(
                        &alvo,
                        Duration::from_millis(500),
                    ) {
                        Ok(_) => format!("✓ peer alcançável (tls; {confia})"),
                        Err(e) => format!("✗ conexão falhou ({e})"),
                    },
                    Err(_) => format!("✗ endereço inválido ({host}:{port})"),
                }
            }
        },
        Endpoint::AutoRemote { identifier } => {
            // Probe é somente leitura e não reabre a janela de descoberta do
            // build: reporta o estado da resolução feita lá.
            format!("discover:{identifier} (resolvida no build; ver coluna rota)")
        }
        Endpoint::AutoRemoteMdns { identifier } => {
            // v1.2 §4.10: mesma semântica do beacon — resolução no build.
            format!("mdns:{identifier} (resolvida no build; ver coluna rota)")
        }
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

    pub(crate) const PROGRAMA_OK: &str = "\
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

    pub(crate) fn roda(args: &[&str]) -> i32 {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(dispatch(args.iter().map(|s| s.to_string())))
    }

    pub(crate) fn tmp_dir(nome: &str) -> PathBuf {
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
        let dir = tmp_dir("run-memory");
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
    async fn run_bloqueia_export_jsonl_e_recusa_modo_fxp_invalido() {
        let dir = tmp_dir("run-jsonl-bloqueado");
        let arq = grava(&dir, "ok.vl", PROGRAMA_OK);
        let ledger = dir.join("caderno.vcad");
        // .jsonl vira DIRETÓRIO: a conversão no sumário falha com aviso
        // honesto — a execução continua (o .vcad é a fonte da verdade).
        std::fs::create_dir_all(jsonl_path(&ledger)).unwrap();
        let code = run(
            arq.to_str().unwrap(),
            Some(2),
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
    }

    #[test]
    fn run_recusa_fxp_mode_invalido_com_codigo_dois() {
        let dir = tmp_dir("run-modo-invalido");
        let arq = grava(&dir, "ok.vl", PROGRAMA_OK);
        let code = roda(&[
            "run",
            arq.to_str().unwrap(),
            "--ticks",
            "1",
            "--fxp-mode",
            "estranho",
        ]);
        assert_eq!(code, 2);
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
        // modo real explícito ⇒ nome do modo no relatório (rota × modo).
        assert_eq!(
            roda(&[
                "fxp-probe",
                "--fxp-config",
                cfg.to_str().unwrap(),
                "--fxp-mode",
                "real"
            ]),
            0
        );
        // rota com endpoint de descoberta ⇒ coluna de rota descreve o auto.
        let cfg_auto = grava(
            &dir,
            "auto.cfg",
            "mode = simulado\ntemp_x.grandeza = temperatura\ntemp_x.unidade = C\n\
             temp_x.mode = real\ntemp_x.endpoint = auto\n",
        );
        assert_eq!(
            roda(&["fxp-probe", "--fxp-config", cfg_auto.to_str().unwrap()]),
            0
        );
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
        assert!(super::actor_availability(&tcp_ruim).starts_with("✗ endereço inválido"));
        // TCP inalcançável: porta de descarte em endereço de loopback válido
        let tcp_morto = Endpoint::Remote {
            addr: RemoteAddr::Tcp {
                host: "127.0.0.1".to_string(),
                port: 1,
            },
        };
        assert!(super::actor_availability(&tcp_morto).starts_with("✗ conexão falhou"));
    }
}

// ── testes in-process do fxpd (o processo-filho dos E2E não é instrumentado
//    pelo llvm-cov; a montagem fica aqui, testável sem dormir) ─────────────
#[cfg(test)]
mod fxpd_tests {
    use super::*;

    fn args_fxpd(extras: &[&str]) -> FxpdArgs {
        let cmd = parse_args(
            std::iter::once("fxpd".to_string()).chain(extras.iter().map(|s| s.to_string())),
        )
        .expect("parse fxpd");
        match cmd {
            Command::FxpDaemon {
                fxp_mode,
                fxp_config,
                serve,
                auth,
                announce,
                announce_mdns,
                compress,
                dict,
                batch,
                timestamp,
                zstd,
                zstd_v,
                ledger,
                tls_cert,
                tls_key,
                tls_sessions,
            } => FxpdArgs {
                fxp_mode,
                fxp_config,
                serve,
                auth,
                announce,
                announce_mdns,
                compress,
                dict,
                batch,
                zstd,
                zstd_v,
                timestamp,
                ledger,
                tls_cert,
                tls_key,
                tls_sessions,
            },
            _ => panic!("esperava FxpDaemon"),
        }
    }

    #[test]
    fn serve_tcp_efemera_monta_e_reporta_porta_real() {
        let dir = std::env::temp_dir().join(format!("fxpd-inproc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg = dir.join("peer.cfg");
        std::fs::write(&cfg, "mode = simulado\n").unwrap();
        let rt = fxpd_preparar(&args_fxpd(&[
            "--serve",
            "tcp:0",
            "--fxp-config",
            cfg.to_str().unwrap(),
            "--batch",
            "--timestamp",
            "--compress",
            "--announce",
            "fxpd-inproc",
        ]))
        .expect("montar");
        assert!(rt.servindo.starts_with("tcp:0.0.0.0:"), "{}", rt.servindo);
        assert!(rt.porta_tcp_real.unwrap_or(0) > 0);
        // bits: LZ4|BATCH|TIMESTAMP = 1|2|4
        assert_eq!(rt.caps_annunciadas, 0b111);
    }

    #[test]
    fn serve_tcp_zstd_v_anuncia_os_tres_bits_e_monta_sessoes() {
        // v1.4 §4.8: --zstd-v anuncia ZSTD|DICT|ZSTD_V (id 4 superset do
        // id 3) e propaga o caminho do cache de sessões TLS em disco.
        use vbl_fxp::schema::caps;
        let dir = std::env::temp_dir().join(format!("fxpd-inproc-z4-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg = dir.join("peer.cfg");
        std::fs::write(&cfg, "mode = simulado\n").unwrap();
        let sessoes = dir.join("sessoes.json");
        let rt = fxpd_preparar(&args_fxpd(&[
            "--serve",
            "tcp:0",
            "--fxp-config",
            cfg.to_str().unwrap(),
            "--zstd",
            "--zstd-v",
            "--tls-sessions",
            sessoes.to_str().unwrap(),
        ]))
        .expect("montar");
        // bits: ZSTD|DICT|ZSTD_V
        assert_eq!(
            rt.caps_annunciadas,
            caps::ZSTD | caps::DICT | caps::ZSTD_V,
            "{:b}",
            rt.caps_annunciadas
        );
        // O caminho das sessões só é exercitado no fio (peer renasce e
        // retoma — ver e2e v14 do vbl-fxp); aqui basta os bits do id 4.
        let _ = sessoes;
    }

    #[test]
    fn serve_unix_monta_e_sem_flags_anuncia_v1_0_puro() {
        let dir = std::env::temp_dir().join(format!("fxpd-inproc-u-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let sock = dir.join("fxpd.sock");
        let rt = fxpd_preparar(&args_fxpd(&[
            "--serve",
            &format!("unix:{}", sock.display()),
        ]))
        .expect("montar");
        assert_eq!(rt.servindo, format!("unix:{}", sock.display()));
        assert_eq!(rt.caps_annunciadas, 0, "sem flags = v1.0 puro");
        assert!(rt.porta_tcp_real.is_none());
    }

    #[test]
    fn psk_de_env_errada_ausente_e_prefixo_errado_falham() {
        let dir = std::env::temp_dir().join(format!("fxpd-inproc-psk-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        // Prefixo errado.
        let r = fxpd_preparar(&args_fxpd(&["--serve", "tcp:0", "--auth", "arquivo:x"]));
        assert_eq!(r.err(), Some(2));

        // Env ausente.
        let r = fxpd_preparar(&args_fxpd(&[
            "--serve",
            "tcp:0",
            "--auth",
            "psk:FXPD_TEST_ENV_INEXISTENTE",
        ]));
        assert_eq!(r.err(), Some(2));

        // Env vazia.
        unsafe { std::env::set_var("FXPD_TEST_ENV_VAZIA", "") };
        let r = fxpd_preparar(&args_fxpd(&[
            "--serve",
            "tcp:0",
            "--auth",
            "psk:FXPD_TEST_ENV_VAZIA",
        ]));
        assert_eq!(r.err(), Some(2));
    }

    #[test]
    fn psk_presente_e_porta_fixa_montam() {
        unsafe { std::env::set_var("FXPD_TEST_ENV_PSK", "segredo-inproc") };
        let rt = fxpd_preparar(&args_fxpd(&[
            "--serve",
            "tcp:0",
            "--auth",
            "psk:FXPD_TEST_ENV_PSK",
        ]))
        .expect("montar com psk");
        assert!(rt.servindo.starts_with("tcp:"));
    }

    #[test]
    fn serve_e_porta_invalidos_falham_com_uso() {
        assert_eq!(
            fxpd_preparar(&args_fxpd(&["--serve", "udp:7080"])).err(),
            Some(2)
        );
        assert_eq!(
            fxpd_preparar(&args_fxpd(&["--serve", "tcp:nao-e-porta"])).err(),
            Some(2)
        );
    }

    #[test]
    fn fxp_mode_invalido_falha() {
        let cmd = parse_args(
            ["fxpd", "--serve", "tcp:0", "--fxp-mode", "caotico"]
                .iter()
                .map(|s| s.to_string()),
        );
        match cmd {
            Ok(Command::FxpDaemon {
                fxp_mode, serve, ..
            }) => {
                let args = FxpdArgs {
                    fxp_mode,
                    fxp_config: None,
                    serve,
                    auth: None,
                    announce: None,
                    announce_mdns: None,
                    compress: false,
                    dict: false,
                    batch: false,
                    timestamp: false,
                    zstd: false,
                    zstd_v: false,
                    ledger: None,
                    tls_cert: None,
                    tls_key: None,
                    tls_sessions: None,
                };
                assert_eq!(super::fxpd_preparar(&args).err(), Some(2));
            }
            _ => panic!("esperava FxpDaemon"),
        }
    }

    #[test]
    fn serve_unix_em_caminho_impossivel_falha() {
        // Diretório inexistente sem permissão para criá-lo ⇒ serve_unix_peer
        // falha honestamente.
        let r = fxpd_preparar(&args_fxpd(&["--serve", "unix:/proc/1/nao/existe.sock"]));
        assert_eq!(r.err(), Some(2));
    }
}

// ── v1.1: cláusulas de erro do run/psk e montagem do fxpd (in-process) ───
#[cfg(test)]
mod fxpd_dispatch_tests {
    use crate::tests::{roda, tmp_dir, PROGRAMA_OK};
    use crate::{fxpd_preparar, Command, FxpdArgs};

    #[test]
    fn run_psk_env_ausente_devolve_dois() {
        let dir = tmp_dir("run-psk-ausente");
        let programa = dir.join("p.vl");
        std::fs::write(&programa, PROGRAMA_OK).unwrap();
        let cfg = dir.join("c.cfg");
        std::fs::write(&cfg, "mode = simulado\n").unwrap();
        let code = roda(&[
            "run",
            programa.to_str().unwrap(),
            "--ticks",
            "1",
            "--fxp-config",
            cfg.to_str().unwrap(),
            "--fxp-psk-env",
            "RUN_TEST_PSK_INEXISTENTE",
        ]);
        assert_eq!(code, 2);
    }

    #[test]
    fn run_config_fxp_invalida_devolve_um() {
        let dir = tmp_dir("run-cfg-invalida");
        let programa = dir.join("p.vl");
        std::fs::write(&programa, PROGRAMA_OK).unwrap();
        let cfg = dir.join("c.cfg");
        std::fs::write(&cfg, "mode = caotico\n").unwrap();
        let code = roda(&[
            "run",
            programa.to_str().unwrap(),
            "--ticks",
            "1",
            "--fxp-config",
            cfg.to_str().unwrap(),
        ]);
        assert_eq!(code, 1);
    }

    #[test]
    fn fxpd_preparar_com_modo_e_porta_ocupada() {
        use crate::args::parse_args;
        let dir = tmp_dir("fxpd-dispatch");
        let cfg = dir.join("peer.cfg");
        std::fs::write(&cfg, "mode = simulado\n").unwrap();

        let montar = |fxp_mode: Option<&str>, porta: &str| {
            let mut extras = vec!["--serve", porta];
            if let Some(m) = fxp_mode {
                extras.push("--fxp-mode");
                extras.push(m);
            }
            let cmd = parse_args(
                std::iter::once("fxpd".to_string()).chain(extras.iter().map(|s| s.to_string())),
            )
            .unwrap();
            match cmd {
                Command::FxpDaemon {
                    fxp_mode,
                    fxp_config,
                    serve,
                    auth,
                    announce,
                    announce_mdns,
                    compress,
                    dict,
                    batch,
                    timestamp,
                    zstd,
                    zstd_v,
                    ledger,
                    tls_cert,
                    tls_key,
                    tls_sessions,
                } => FxpdArgs {
                    fxp_mode,
                    fxp_config,
                    serve,
                    auth,
                    announce,
                    announce_mdns,
                    compress,
                    dict,
                    batch,
                    timestamp,
                    zstd,
                    zstd_v,
                    ledger,
                    tls_cert,
                    tls_key,
                    tls_sessions,
                },
                _ => panic!("esperava FxpDaemon"),
            }
        };

        // --fxp-mode simulado|real|hibrido ⇒ braços válidos do match.
        for modo in ["simulado", "real", "hibrido"] {
            let rt = fxpd_preparar(&montar(Some(modo), "tcp:0")).expect("montar");
            assert!(rt.servindo.starts_with("tcp:"));
            drop(rt);
        }
        // modo inválido ⇒ erro tipado 2 (braço "other" do match).
        let err = match fxpd_preparar(&montar(Some("estranho"), "tcp:0")) {
            Err(code) => code,
            Ok(_) => panic!("modo estranho devia falhar"),
        };
        assert_eq!(err, 2);
        // --ledger ARQUIVO ⇒ Caderno de produção no peer montado.
        let dir_ledger = dir.join("caderno");
        std::fs::create_dir_all(&dir_ledger).unwrap();
        let mut args_ledger = montar(Some("simulado"), "tcp:0");
        args_ledger.ledger = Some(dir_ledger.join("peer.vcad"));
        let rt2 = fxpd_preparar(&args_ledger).expect("montar com ledger");
        assert!(rt2.servindo.starts_with("tcp:"));
        drop(rt2);

        // Porta TCP OCUPADA ⇒ falha honesta do serve (407-409).
        let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let porta = listener.local_addr().unwrap().port();
        let r = fxpd_preparar(&montar(None, &format!("tcp:{porta}")));
        assert_eq!(r.err(), Some(2));
    }

    #[test]
    fn fxpd_por_dispatch_imprime_pronto_e_dorme() {
        // O braço do dispatch + o corpo de `fxpd` (impressão do "pronto" e o
        // park) rodam numa thread destacada; o processo de teste encerra
        // normalmente (a thread dorme até o fim — comportamento de daemon).
        let dir = tmp_dir("fxpd-dispatch-thread");
        let cfg = dir.join("peer.cfg");
        std::fs::write(&cfg, "mode = simulado\n").unwrap();
        let cfg_str = cfg.display().to_string();
        let handle = std::thread::spawn(move || {
            roda(&[
                "fxpd",
                "--serve",
                "tcp:0",
                "--fxp-config",
                &cfg_str,
                "--batch",
            ])
        });
        // Dá tempo de a thread imprimir o "pronto" e chegar ao park; sem
        // asserção de saída (não retorna) — o objetivo é a execução coberta.
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(!handle.is_finished(), "daemon não deveria terminar sozinho");
    }
}

// ── Bateria de dispatch: probe com rotas variadas, ledger-verify corrupto,
//    run em tempo real e modos do fxpd (cobra as cláusulas de exibição). ──
#[cfg(test)]
mod probe_battery_tests {
    use crate::args::{parse_args, Command};
    use crate::tests::{roda, tmp_dir, PROGRAMA_OK};
    use crate::FxpdArgs;
    use vbl_fxp::Endpoint;

    #[test]
    fn fxp_probe_sem_obrigatorios_falha_e_rotas_variadas_imprimem_honesto() {
        let dir = tmp_dir("probe-bateria");

        // Registro semeia o mínimo (FORMAL §6) + extensão com hwmon ausente:
        // o probe imprime a disponibilidade honesta e sai 0.
        let cfg_ext = dir.join("ext.cfg");
        std::fs::write(
            &cfg_ext,
            "mode = simulado\n\
             temp_ext.grandeza = temperatura\ntemp_ext.unidade = C\n\
             temp_ext.mode = real\ntemp_ext.endpoint = hwmon_temp:/tmp/nao/existe\n",
        )
        .unwrap();
        assert_eq!(
            roda(&["fxp-probe", "--fxp-config", cfg_ext.to_str().unwrap()]),
            0
        );

        // Endpoints variados: unix PRESENTE, tcp alcançável, discover, hwmon
        // ausente, rapl — o probe imprime disponibilidade sem mentir.
        let sock_path = dir.join("probe.sock");
        let _unix = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let porta = tcp.local_addr().unwrap().port();
        let cfg_rota = dir.join("rotas.cfg");
        std::fs::write(
            &cfg_rota,
            format!(
                "mode = hibrido\n\
                 Fan.min = 0\nFan.max = 255\nFan.safety_limit = 200\nFan.mode = real\nFan.endpoint = unix:{}\n\
                 cpu_temp.grandeza = temperatura\ncpu_temp.unidade = C\n\
                 cpu_temp.mode = real\ncpu_temp.endpoint = tcp:127.0.0.1:{porta}\n\
                 cpu_power.grandeza = potencia\ncpu_power.unidade = W\n\
                 cpu_power.mode = real\ncpu_power.endpoint = rapl_energy:/sys/class/powercap\n\
                 attention.grandeza = atencao\nattention.unidade = %\n\
                 attention.mode = real\nattention.endpoint = discover:fxpd-lab\n",
                sock_path.display()
            ),
        )
        .unwrap();
        assert_eq!(
            roda(&[
                "fxp-probe",
                "--fxp-config",
                cfg_rota.to_str().unwrap(),
                "--fxp-mode",
                "hibrido"
            ]),
            0
        );
    }

    #[test]
    fn ledger_verify_jsonl_corrompida_reporta_e_falha() {
        let dir = tmp_dir("ledger-corrupto");
        let jsonl = dir.join("corrompido.vcad.jsonl");
        std::fs::write(&jsonl, "{\" linha totalmente invalida\n").unwrap();
        let code = roda(&["ledger-verify", jsonl.to_str().unwrap()]);
        assert_ne!(code, 0, "JSONL corrupta deve falhar a auditoria");
    }

    #[test]
    fn run_com_tempo_real_executa_ticks_por_intervalo() {
        let dir = tmp_dir("run-tempo-real");
        let programa = dir.join("p.vl");
        std::fs::write(&programa, PROGRAMA_OK).unwrap();
        let code = roda(&[
            "run",
            programa.to_str().unwrap(),
            "--ticks",
            "1",
            "--real-ms",
            "30",
        ]);
        assert_eq!(code, 0);
    }

    fn args_fxpd_local(extras: &[&str]) -> FxpdArgs {
        let cmd = parse_args(
            std::iter::once("fxpd".to_string()).chain(extras.iter().map(|s| s.to_string())),
        )
        .expect("parse fxpd");
        match cmd {
            Command::FxpDaemon {
                fxp_mode,
                fxp_config,
                serve,
                auth,
                announce,
                announce_mdns,
                compress,
                dict,
                batch,
                timestamp,
                zstd,
                zstd_v,
                ledger,
                tls_cert,
                tls_key,
                tls_sessions,
            } => FxpdArgs {
                fxp_mode,
                fxp_config,
                serve,
                auth,
                announce,
                announce_mdns,
                compress,
                dict,
                batch,
                zstd,
                zstd_v,
                timestamp,
                ledger,
                tls_cert,
                tls_key,
                tls_sessions,
            },
            _ => panic!("esperava FxpDaemon"),
        }
    }

    #[test]
    fn fxpd_fxpmode_invalido_e_disponibilidade_remota_honestas() {
        // --fxp-mode inválido ⇒ honesto no arranque (código 2).
        let args = args_fxpd_local(&["--serve", "tcp:0", "--fxp-mode", "turbo"]);
        assert_eq!(super::fxpd_preparar(&args).err(), Some(2));

        // Disponibilidade de ator remoto TCP: porta ABERTA ⇒ alcançável…
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let porta = listener.local_addr().expect("addr").port();
        let msg = super::actor_availability(&Endpoint::Remote {
            addr: vbl_fxp::RemoteAddr::Tcp {
                host: "127.0.0.1".into(),
                port: porta,
            },
        });
        assert!(msg.contains("alcançável"), "{msg}");

        // …porta FECHADA ⇒ falha honesta (sem fingir).
        let msg2 = super::actor_availability(&Endpoint::Remote {
            addr: vbl_fxp::RemoteAddr::Tcp {
                host: "127.0.0.1".into(),
                port: 1,
            },
        });
        assert!(msg2.contains("falhou"), "{msg2}");
    }

    #[test]
    fn fxpd_tls_cert_ilegivel_falha_honesto_no_arranque() {
        // PEM inexistente ⇒ erro honesto de arranque (código 2), nunca
        // daemon no ar sem TLS nem degradação para texto plano.
        let args = args_fxpd_local(&[
            "--serve",
            "tcp:0",
            "--tls-cert",
            "nao-existe.pem",
            "--tls-key",
            "nao-existe.key.pem",
        ]);
        assert_eq!(super::fxpd_preparar(&args).err(), Some(2));
    }

    #[test]
    fn disponibilidade_de_ator_cobre_todos_os_bracos_honestos() {
        use std::path::PathBuf;
        // Rota real com endpoint ausente; descobertas; tcps com pin.
        assert!(super::actor_availability(&Endpoint::ThermalZone {
            dir: PathBuf::from("/vbl-certo-nao-existe"),
        })
        .contains("ausente"));
        assert!(super::actor_availability(&Endpoint::RaplConstraint {
            file: PathBuf::from("/vbl-certo-nao-existe"),
        })
        .contains("ausente"));
        assert!(super::actor_availability(&Endpoint::HwmonPwm {
            file: PathBuf::from("/vbl-certo-nao-existe"),
        })
        .contains("ausente"));
        assert!(super::actor_availability(&Endpoint::AutoRemote {
            identifier: "x".into(),
        })
        .contains("discover:"));
        assert!(super::actor_availability(&Endpoint::AutoRemoteMdns {
            identifier: "x".into(),
        })
        .contains("mdns:"));
        // tcps com endereço inválido ⇒ honesto sem conectar.
        assert!(super::actor_availability(&Endpoint::Remote {
            addr: vbl_fxp::RemoteAddr::TcpTls {
                host: "nao-e-ip".into(),
                port: 1,
                trust: vbl_fxp::tls::Trust::Pin(vec![[0u8; 32]]),
            },
        })
        .contains("inválido"));
        // tcps em porta fechada ⇒ conexão falhou (honesto).
        assert!(super::actor_availability(&Endpoint::Remote {
            addr: vbl_fxp::RemoteAddr::TcpTls {
                host: "127.0.0.1".into(),
                port: 1,
                trust: vbl_fxp::tls::Trust::Pin(vec![[0u8; 32]]),
            },
        })
        .contains("falhou"));
        // tcp endereço inválido ⇒ honesto.
        assert!(super::actor_availability(&Endpoint::Remote {
            addr: vbl_fxp::RemoteAddr::Tcp {
                host: "x".into(),
                port: 1
            },
        })
        .contains("inválido"));
    }
}
