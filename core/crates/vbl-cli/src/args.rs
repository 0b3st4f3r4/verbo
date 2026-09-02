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
    },
    /// `vbl fxp-probe` — tabela de dispositivos/modos/rotas/disponibilidade
    /// (auditoria do registro FXP no host; PLAN Etapa 3).
    FxpProbe {
        fxp_mode: Option<String>,
        fxp_config: Option<PathBuf>,
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
            while let Some(a) = args.next() {
                match a.as_str() {
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
  vbl ledger-verify <ARQUIVO>

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
}
