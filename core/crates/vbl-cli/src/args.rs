//! Argumentos de linha de comando do `vbl` (sem dependências externas).

use crate::script::Script;
use std::path::PathBuf;

// Variante `Run` carrega as opções de execução; o enum vive poucos ciclos no
// início do processo — a clareza dos campos nomeados vale mais que o boxing.
#[allow(clippy::large_enum_variant)]
pub enum Command {
    Check {
        arquivo: String,
        with_registry: bool,
    },
    Run {
        arquivo: String,
        ticks: Option<u64>,
        real_ms: Option<u64>,
        persist_dir: PathBuf,
        ledger: Option<PathBuf>,
        script: Script,
        allow_unregistered: bool,
        /// Modo do barramento FXP (sobrepõe o `mode` do arquivo de config).
        fxp_mode: Option<String>,
        /// Arquivo de registro/config FXP (`key = value`, docs/FXP-SCHEMA-v1.md §7).
        fxp_config: Option<PathBuf>,
        /// PSK do cliente remoto (§4.6): lida da env `VAR` — a chave nunca
        /// trafega nem fica em arquivo.
        fxp_psk_env: Option<String>,
    },
    /// `vbl fxp-probe` — tabela de dispositivos/modos/rotas/disponibilidade
    /// (auditoria do registro FXP no host; PLAN Etapa 3).
    FxpProbe {
        fxp_mode: Option<String>,
        fxp_config: Option<PathBuf>,
    },
    /// `vbl fxpd` — o peer FXP v1.1 (schema §7: servidor de referência):
    /// AUTH → CAPS → trabalho; recursos v1.1 opt-in (PLAN §8 item 8).
    FxpDaemon {
        fxp_mode: Option<String>,
        fxp_config: Option<PathBuf>,
        /// `unix:PATH` ou `tcp:PORTA` (porta 0 = efêmera, impressa no pronto).
        serve: String,
        /// PSK de env (`--auth psk:NOME_DA_VAR`) — §4.6; handshake obrigatório.
        auth: Option<String>,
        /// Anuncia o beacon multicast FXPD (§4.9) com este identificador.
        announce: Option<String>,
        /// Anuncia LZ4 (§4.8) — compressão negociada das respostas.
        compress: bool,
        /// Anuncia BATCH (§4.7) — lote de leituras.
        batch: bool,
        /// Anuncia TIMESTAMP (§5) — carimbo físico nas respostas.
        timestamp: bool,
        /// Caderno do peer (produção); sem ele, o Caderno fica desligado
        /// (aviso honesto — §4.7 não registra eventos sem Caderno).
        ledger: Option<PathBuf>,
    },
    /// `vbl ledger-verify ARQUIVO` — verificação externa do log do Caderno
    /// (binário `.vcad` ou JSONL): recomputa a cadeia SHA-256 e emite o
    /// relatório (Etapa 4 — AGENTS §1.4: agente externo).
    LedgerVerify { arquivo: String },
}

pub fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let sub = args.next().ok_or(USAGE)?;
    match sub.as_str() {
        "check" => {
            let mut arquivo = None;
            let mut with_registry = true;
            for a in args.by_ref() {
                match a.as_str() {
                    "--no-registry" => with_registry = false,
                    _ if arquivo.is_none() => arquivo = Some(a),
                    _ => return Err(format!("argumento inesperado: {a}\n{USAGE}")),
                }
            }
            Ok(Command::Check {
                arquivo: arquivo.ok_or(format!("check exige <arquivo.vl>\n{USAGE}"))?,
                with_registry,
            })
        }
        "fxpd" => {
            let mut fxp_mode = None;
            let mut fxp_config = None;
            let mut serve: Option<String> = None;
            let mut auth = None;
            let mut announce = None;
            let mut compress = false;
            let mut batch = false;
            let mut timestamp = false;
            let mut ledger = None;
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--fxp-mode" => {
                        fxp_mode = Some(
                            args.next()
                                .ok_or("--fxp-mode exige simulado|real|hibrido")?,
                        )
                    }
                    "--fxp-config" => {
                        fxp_config = Some(PathBuf::from(
                            args.next().ok_or("--fxp-config exige ARQUIVO")?,
                        ))
                    }
                    "--serve" => {
                        serve = Some(
                            args.next()
                                .ok_or("--serve exige unix:PATH|tcp:PORTA")?,
                        )
                    }
                    "--auth" => {
                        auth = Some(args.next().ok_or("--auth exige psk:VAR_DE_ENV")?)
                    }
                    "--announce" => {
                        announce = Some(args.next().ok_or("--announce exige IDENTIFICADOR")?)
                    }
                    "--compress" => compress = true,
                    "--batch" => batch = true,
                    "--timestamp" => timestamp = true,
                    "--ledger" => {
                        ledger = Some(PathBuf::from(
                            args.next().ok_or("--ledger exige ARQUIVO")?,
                        ))
                    }
                    other => return Err(format!("argumento de fxpd inesperado: {other}\n{USAGE}")),
                }
            }
            Ok(Command::FxpDaemon {
                fxp_mode,
                fxp_config,
                serve: serve.ok_or(format!("fxpd exige --serve unix:PATH|tcp:PORTA\n{USAGE}"))?,
                auth,
                announce,
                compress,
                batch,
                timestamp,
                ledger,
            })
        }
        "run" => {
            let mut arquivo = None;
            let mut ticks = None;
            let mut real_ms = None;
            let mut persist_dir = PathBuf::from("persistence");
            let mut ledger = None;
            let mut script = Script::default();
            let mut allow_unregistered = false;
            let mut fxp_mode = None;
            let mut fxp_config = None;
            let mut fxp_psk_env = None;
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--fxp-psk-env" => {
                        fxp_psk_env = Some(args.next().ok_or("--fxp-psk-env exige VAR")?)
                    }
                    "--ticks" => {
                        ticks = Some(
                            args.next()
                                .ok_or("--ticks exige N")?
                                .parse()
                                .map_err(|_| "--ticks exige inteiro")?,
                        )
                    }
                    "--real-ms" => {
                        real_ms = Some(
                            args.next()
                                .ok_or("--real-ms exige MS")?
                                .parse()
                                .map_err(|_| "--real-ms exige inteiro")?,
                        )
                    }
                    "--persist-dir" => {
                        persist_dir = PathBuf::from(args.next().ok_or("--persist-dir exige DIR")?)
                    }
                    "--ledger" => {
                        ledger = Some(PathBuf::from(args.next().ok_or("--ledger exige ARQUIVO")?))
                    }
                    "--set" => {
                        let kv = args.next().ok_or("--set exige SENSOR=VALOR")?;
                        let (sensor, value) =
                            kv.split_once('=').ok_or("--set espera SENSOR=VALOR")?;
                        script.set(
                            sensor,
                            value
                                .parse()
                                .map_err(|_| "valor do sensor deve ser número")?,
                        );
                    }
                    "--at" => {
                        let kv = args.next().ok_or("--at exige TICK:SENSOR=VALOR")?;
                        let (tick, rest) =
                            kv.split_once(':').ok_or("--at espera TICK:SENSOR=VALOR")?;
                        let (sensor, value) = rest
                            .split_once('=')
                            .ok_or("--at espera TICK:SENSOR=VALOR")?;
                        script.at(
                            tick.parse().map_err(|_| "tick deve ser inteiro")?,
                            sensor,
                            value
                                .parse()
                                .map_err(|_| "valor do sensor deve ser número")?,
                        );
                    }
                    "--allow-unregistered" => allow_unregistered = true,
                    "--fail-actor" => {
                        script.fail_actor(&args.next().ok_or("--fail-actor exige NOME")?)
                    }
                    "--fallback" => {
                        let kv = args.next().ok_or("--fallback exige PRIMARIO=ALTERNATIVO")?;
                        let (prim, alt) = kv
                            .split_once('=')
                            .ok_or("--fallback espera PRIMARIO=ALTERNATIVO")?;
                        script.fallback(prim, alt);
                    }
                    "--register-actor" => {
                        script.register_actor(&args.next().ok_or("--register-actor exige NOME")?)
                    }
                    "--fxp-mode" => {
                        fxp_mode = Some(
                            args.next()
                                .ok_or("--fxp-mode exige simulado|real|hibrido")?,
                        )
                    }
                    "--fxp-config" => {
                        fxp_config = Some(PathBuf::from(
                            args.next().ok_or("--fxp-config exige ARQUIVO")?,
                        ))
                    }
                    _ if arquivo.is_none() => arquivo = Some(a),
                    _ => return Err(format!("argumento inesperado: {a}\n{USAGE}")),
                }
            }
            Ok(Command::Run {
                arquivo: arquivo.ok_or(format!("run exige <arquivo.vl>\n{USAGE}"))?,
                ticks,
                real_ms,
                persist_dir,
                ledger,
                script,
                allow_unregistered,
                fxp_mode,
                fxp_config,
                fxp_psk_env,
            })
        }
        "fxp-probe" => {
            let mut fxp_mode = None;
            let mut fxp_config = None;
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--fxp-mode" => {
                        fxp_mode = Some(
                            args.next()
                                .ok_or("--fxp-mode exige simulado|real|hibrido")?,
                        )
                    }
                    "--fxp-config" => {
                        fxp_config = Some(PathBuf::from(
                            args.next().ok_or("--fxp-config exige ARQUIVO")?,
                        ))
                    }
                    _ => return Err(format!("argumento inesperado: {a}\n{USAGE}")),
                }
            }
            Ok(Command::FxpProbe {
                fxp_mode,
                fxp_config,
            })
        }
        "ledger-verify" => {
            let mut arquivo = None;
            for a in args.by_ref() {
                if arquivo.is_none() {
                    arquivo = Some(a);
                } else {
                    return Err(format!("argumento inesperado: {a}\n{USAGE}"));
                }
            }
            Ok(Command::LedgerVerify {
                arquivo: arquivo.ok_or(format!("ledger-verify exige <ARQUIVO>\n{USAGE}"))?,
            })
        }
        "--help" | "-h" | "help" => Err(USAGE.to_string()),
        other => Err(format!("subcomando desconhecido '{other}'\n{USAGE}")),
    }
}

const USAGE: &str = "\
uso:
  vbl check <arquivo.vl> [--no-registry]
  vbl run <arquivo.vl> [opções]
  vbl fxp-probe [--fxp-config ARQUIVO] [--fxp-mode MODO]
  vbl fxpd --serve unix:PATH|tcp:PORTA [opções]
  vbl ledger-verify <ARQUIVO>

opções de fxpd (schema v1.1 — docs/FXP-SCHEMA-v1.md §7/§4.5–§4.9):
  --fxp-config ARQUIVO             registro/config FXP servido pelo peer
  --fxp-mode MODO                  simulado|real|hibrido (padrão: o da config, senão simulado)
  --serve unix:PATH|tcp:PORTA      transporte do peer (porta 0 = efêmera)
  --auth psk:VAR_DE_ENV            PSK (§4.6): handshake AUTH obrigatório; chave NUNCA em arquivo
  --announce IDENTIFICADOR         beacon multicast FXPD (§4.9) com este identificador
  --compress                       anuncia LZ4 (§4.8)
  --batch                          anuncia READ_BATCH (§4.7)
  --timestamp                      anuncia FLAG_TIMESTAMP (§5 — carimbo físico)
  --ledger ARQUIVO                 Caderno do peer (produção .vcad); sem ele, desligado

opções de run:
  --ticks N                        número de ticks virtuais (padrão: até esvaziar o mundo)
  --real-ms MS                     modo tempo real: 1 tick a cada MS milissegundos
  --persist-dir DIR                diretório de persistência `.vl` (padrão: persistence/)
  --ledger ARQUIVO                 Caderno de PRODUÇÃO (Etapa 4): gravação assíncrona;
                                   binário .vcad em ARQUIVO + JSONL em ARQUIVO.jsonl
  --set SENSOR=VALOR               valor inicial de sensor no FXP simulado
  --at TICK:SENSOR=VALOR           roteiriza valor absoluto de sensor no tick
  --fail-actor NOME                ator para de responder (heartbeat — BDD Caso 3)
  --fallback PRIM=ALT              política de fallback do registro (FORMAL §4.3)
  --register-actor NOME            ator extra 0..255 safety 200 (ex.: ReserveFan)
  --allow-unregistered             executa mesmo com referências fora do registro (§4.7)
  --fxp-mode MODO                  simulado|real|hibrido (padrão: simulado; sobrepõe a config)
  --fxp-config ARQUIVO             registro/config FXP (dispositivos, endpoints, fallback)
  --fxp-psk-env VAR                PSK do cliente remoto (§4.6): chave vem da env VAR

opções de fxp-probe:
  --fxp-config ARQUIVO             registro/config FXP a auditar
  --fxp-mode MODO                  simulado|real|hibrido (padrão: o da config, senão simulado)
";

// ── suíte do parser de argumentos (uso, flags e cláusulas de erro) ────────
#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> std::vec::IntoIter<String> {
        list.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn sem_subcomando_devolve_uso() {
        assert!(parse_args(args(&[])).is_err());
    }

    #[test]
    fn posicionais_extras_sao_rejeitados() {
        // run com dois arquivos, probe com posicional e check com posicional
        // extra: todos caem em "argumento inesperado" com o USAGE.
        for caso in [
            vec!["run", "a.vl", "b.vl"],
            vec!["fxp-probe", "estranho"],
            vec!["check", "a.vl", "b.vl"],
            vec!["ledger-verify", "a.vcad", "b.vcad"],
        ] {
            let err = match parse_args(args(&caso)) { Err(e) => e, Ok(_) => panic!("{caso:?} devia falhar") };
            assert!(err.contains("argumento inesperado"), "{caso:?}: {err}");
            assert!(err.contains("USO") || err.contains("uso"), "{err}");
        }
    }

    #[test]
    fn check_flags() {
        let Command::Check {
            arquivo,
            with_registry,
        } = parse_args(args(&["check", "a.vl"])).unwrap()
        else {
            panic!("esperado Check")
        };
        assert_eq!(arquivo, "a.vl");
        assert!(with_registry); // padrão: valida contra o registro
        let Command::Check {
            arquivo,
            with_registry,
        } = parse_args(args(&["check", "--no-registry", "b.vl"])).unwrap()
        else {
            panic!("esperado Check")
        };
        assert_eq!(arquivo, "b.vl");
        assert!(!with_registry);
        // dois arquivos → argumento inesperado
        assert!(parse_args(args(&["check", "a.vl", "b.vl"])).is_err());
        // sem arquivo
        assert!(parse_args(args(&["check"])).is_err());
    }

    #[test]
    fn run_flags_completos() {
        let Command::Run {
            arquivo,
            ticks,
            real_ms,
            persist_dir,
            ledger,
            script,
            allow_unregistered,
            fxp_mode,
            fxp_config,
            fxp_psk_env: _,
        } = parse_args(args(&[
            "run",
            "p.vl",
            "--ticks",
            "7",
            "--real-ms",
            "100",
            "--persist-dir",
            "/tmp/pp",
            "--ledger",
            "log.vcad",
            "--set",
            "cpu_temp=90",
            "--at",
            "3:attention=15",
            "--fail-actor",
            "Fan",
            "--fallback",
            "Fan=ReserveFan",
            "--register-actor",
            "ReserveFan",
            "--allow-unregistered",
            "--fxp-mode",
            "hibrido",
            "--fxp-config",
            "fxp.cfg",
        ]))
        .unwrap()
        else {
            panic!("esperado Run")
        };
        assert_eq!(arquivo, "p.vl");
        assert_eq!(ticks, Some(7));
        assert_eq!(real_ms, Some(100));
        assert_eq!(persist_dir, PathBuf::from("/tmp/pp"));
        assert_eq!(ledger, Some(PathBuf::from("log.vcad")));
        assert!(allow_unregistered);
        assert_eq!(fxp_mode.as_deref(), Some("hibrido"));
        assert_eq!(fxp_config, Some(PathBuf::from("fxp.cfg")));
        // o roteiro carregou tudo
        use vbl_runtime::fxp::Fxp as _;
        let mut sim = script.build_simulator();
        let mut ledger = vbl_runtime::ledger::ChainLedger::new();
        assert_eq!(sim.read_sensor("cpu_temp", &mut ledger), Ok(90.0)); // valor inicial
    }

    #[test]
    fn run_clausulas_de_erro_dos_valores() {
        // flags de valor ausente
        assert!(parse_args(args(&["run", "a.vl", "--ticks"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--real-ms"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--persist-dir"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--ledger"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--set"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--at"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--fail-actor"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--fallback"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--register-actor"])).is_err());
        // valores malformados
        assert!(parse_args(args(&["run", "a.vl", "--ticks", "sete"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--real-ms", "rápido"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--set", "sem_igual"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--set", "cpu_temp=quente"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--at", "sem_dois_pontos"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--at", "1:cpu_temp=forte"])).is_err());
        assert!(parse_args(args(&["run", "a.vl", "--fallback", "so_um"])).is_err());
        // subcomando desconhecido
        assert!(parse_args(args(&["voar"])).is_err());
    }

    #[test]
    fn fxp_probe_e_ledger_verify() {
        let Command::FxpProbe {
            fxp_mode,
            fxp_config,
        } = parse_args(args(&[
            "fxp-probe",
            "--fxp-mode",
            "real",
            "--fxp-config",
            "c.cfg",
        ]))
        .unwrap()
        else {
            panic!("esperado FxpProbe")
        };
        assert_eq!(fxp_mode.as_deref(), Some("real"));
        assert_eq!(fxp_config, Some(PathBuf::from("c.cfg")));
        let Command::LedgerVerify { arquivo } =
            parse_args(args(&["ledger-verify", "log.vcad"])).unwrap()
        else {
            panic!("esperado LedgerVerify")
        };
        assert_eq!(arquivo, "log.vcad");
        assert!(parse_args(args(&["ledger-verify"])).is_err());
    }

    #[test]
    fn fxpd_flags_completos_e_clausulas_de_erro() {
        let Command::FxpDaemon {
            fxp_mode,
            fxp_config,
            serve,
            auth,
            announce,
            compress,
            batch,
            timestamp,
            ledger,
        } = parse_args(args(&[
            "fxpd",
            "--serve",
            "tcp:7080",
            "--fxp-config",
            "p.cfg",
            "--fxp-mode",
            "hibrido",
            "--auth",
            "psk:MINHA_VAR",
            "--announce",
            "fxpd-lab",
            "--compress",
            "--batch",
            "--timestamp",
            "--ledger",
            "peer.vcad",
        ]))
        .unwrap()
        else {
            panic!("esperado FxpDaemon")
        };
        assert_eq!(serve, "tcp:7080");
        assert_eq!(fxp_config, Some(PathBuf::from("p.cfg")));
        assert_eq!(fxp_mode.as_deref(), Some("hibrido"));
        assert_eq!(auth.as_deref(), Some("psk:MINHA_VAR"));
        assert_eq!(announce.as_deref(), Some("fxpd-lab"));
        assert!(compress && batch && timestamp);
        assert_eq!(ledger, Some(PathBuf::from("peer.vcad")));

        // Cláusulas de erro: sem --serve, argumento estranho, flags que
        // exigem valor sem o valor.
        assert!(parse_args(args(&["fxpd"])).is_err());
        assert!(parse_args(args(&["fxpd", "--turbo"])).is_err());
        assert!(parse_args(args(&["fxpd", "--serve"])).is_err());
        assert!(parse_args(args(&["fxpd", "--auth"])).is_err());
        assert!(parse_args(args(&["fxpd", "--announce"])).is_err());
        assert!(parse_args(args(&["fxpd", "--ledger"])).is_err());
        assert!(parse_args(args(&["fxpd", "--fxp-config"])).is_err());
        assert!(parse_args(args(&["fxpd", "--fxp-mode"])).is_err());
    }

    #[test]
    fn run_psk_env_flag_e_erro_sem_valor() {
        let Command::Run {
            fxp_psk_env,
            fxp_config,
            ..
        } = parse_args(args(&[
            "run",
            "p.vl",
            "--fxp-psk-env",
            "VAR_PSK",
            "--fxp-config",
            "c.cfg",
        ]))
        .unwrap()
        else {
            panic!("esperado Run")
        };
        assert_eq!(fxp_psk_env.as_deref(), Some("VAR_PSK"));
        assert_eq!(fxp_config, Some(PathBuf::from("c.cfg")));
        assert!(parse_args(args(&["run", "p.vl", "--fxp-psk-env"])).is_err());
    }
}
