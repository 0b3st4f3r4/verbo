//! Argumentos de linha de comando do `vbl` (sem dependências externas).

use std::path::PathBuf;
use crate::script::Script;

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
    LedgerVerify {
        arquivo: String,
    },
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
            let mut persist_dir = PathBuf::from("persistencia");
            let mut ledger = None;
            let mut script = Script::default();
            let mut allow_unregistered = false;
            let mut fxp_mode = None;
            let mut fxp_config = None;
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--ticks" => {
                        ticks = Some(args.next().ok_or("--ticks exige N")?.parse().map_err(|_| "--ticks exige inteiro")?)
                    }
                    "--real-ms" => {
                        real_ms = Some(args.next().ok_or("--real-ms exige MS")?.parse().map_err(|_| "--real-ms exige inteiro")?)
                    }
                    "--persist-dir" => {
                        persist_dir = PathBuf::from(args.next().ok_or("--persist-dir exige DIR")?)
                    }
                    "--ledger" => {
                        ledger = Some(PathBuf::from(args.next().ok_or("--ledger exige ARQUIVO")?))
                    }
                    "--set" => {
                        let kv = args.next().ok_or("--set exige SENSOR=VALOR")?;
                        let (sensor, value) = kv.split_once('=').ok_or("--set espera SENSOR=VALOR")?;
                        script.set(sensor, value.parse().map_err(|_| "valor do sensor deve ser número")?);
                    }
                    "--at" => {
                        let kv = args.next().ok_or("--at exige TICK:SENSOR=VALOR")?;
                        let (tick, rest) = kv.split_once(':').ok_or("--at espera TICK:SENSOR=VALOR")?;
                        let (sensor, value) = rest.split_once('=').ok_or("--at espera TICK:SENSOR=VALOR")?;
                        script.at(
                            tick.parse().map_err(|_| "tick deve ser inteiro")?,
                            sensor,
                            value.parse().map_err(|_| "valor do sensor deve ser número")?,
                        );
                    }
                    "--allow-unregistered" => allow_unregistered = true,
                    "--fail-actor" => {
                        script.fail_actor(&args.next().ok_or("--fail-actor exige NOME")?)
                    }
                    "--fallback" => {
                        let kv = args.next().ok_or("--fallback exige PRIMARIO=ALTERNATIVO")?;
                        let (prim, alt) =
                            kv.split_once('=').ok_or("--fallback espera PRIMARIO=ALTERNATIVO")?;
                        script.fallback(prim, alt);
                    }
                    "--register-actor" => {
                        script.register_actor(&args.next().ok_or("--register-actor exige NOME")?)
                    }
                    "--fxp-mode" => {
                        fxp_mode = Some(args.next().ok_or("--fxp-mode exige simulado|real|hibrido")?)
                    }
                    "--fxp-config" => {
                        fxp_config = Some(PathBuf::from(args.next().ok_or("--fxp-config exige ARQUIVO")?))
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
                        fxp_mode = Some(args.next().ok_or("--fxp-mode exige simulado|real|hibrido")?)
                    }
                    "--fxp-config" => {
                        fxp_config = Some(PathBuf::from(args.next().ok_or("--fxp-config exige ARQUIVO")?))
                    }
                    _ => return Err(format!("argumento inesperado: {a}\n{USAGE}")),
                }
            }
            Ok(Command::FxpProbe { fxp_mode, fxp_config })
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
  --persist-dir DIR                diretório de persistência `.vl` (padrão: persistencia/)
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
