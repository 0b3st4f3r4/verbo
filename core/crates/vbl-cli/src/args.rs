//! Argumentos de linha de comando do `vbl` (sem dependências externas).

use std::path::PathBuf;
use crate::script::Roteiro;

// Variante `Run` carrega as opções de execução; o enum vive poucos ciclos no
// início do processo — a clareza dos campos nomeados vale mais que o boxing.
#[allow(clippy::large_enum_variant)]
pub enum Comando {
    Check {
        arquivo: String,
        com_registro: bool,
    },
    Run {
        arquivo: String,
        ticks: Option<u64>,
        real_ms: Option<u64>,
        persist_dir: PathBuf,
        caderno: Option<PathBuf>,
        roteiro: Roteiro,
        permitir_sem_registro: bool,
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
    /// `vbl caderno-verify ARQUIVO` — verificação externa do log do Caderno
    /// (binário `.vcad` ou JSONL): recomputa a cadeia SHA-256 e emite o
    /// relatório (Etapa 4 — AGENTS §1.4: agente externo).
    CadernoVerify {
        arquivo: String,
    },
}

pub fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Comando, String> {
    let sub = args.next().ok_or(USO)?;
    match sub.as_str() {
        "check" => {
            let mut arquivo = None;
            let mut com_registro = true;
            for a in args.by_ref() {
                match a.as_str() {
                    "--sem-registro" => com_registro = false,
                    _ if arquivo.is_none() => arquivo = Some(a),
                    _ => return Err(format!("argumento inesperado: {a}\n{USO}")),
                }
            }
            Ok(Comando::Check {
                arquivo: arquivo.ok_or(format!("check exige <arquivo.vl>\n{USO}"))?,
                com_registro,
            })
        }
        "run" => {
            let mut arquivo = None;
            let mut ticks = None;
            let mut real_ms = None;
            let mut persist_dir = PathBuf::from("persistencia");
            let mut caderno = None;
            let mut roteiro = Roteiro::default();
            let mut permitir_sem_registro = false;
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
                    "--caderno" => {
                        caderno = Some(PathBuf::from(args.next().ok_or("--caderno exige ARQUIVO")?))
                    }
                    "--set" => {
                        let kv = args.next().ok_or("--set exige SENSOR=VALOR")?;
                        let (sensor, valor) = kv.split_once('=').ok_or("--set espera SENSOR=VALOR")?;
                        roteiro.set(sensor, valor.parse().map_err(|_| "valor do sensor deve ser número")?);
                    }
                    "--at" => {
                        let kv = args.next().ok_or("--at exige TICK:SENSOR=VALOR")?;
                        let (tick, resto) = kv.split_once(':').ok_or("--at espera TICK:SENSOR=VALOR")?;
                        let (sensor, valor) = resto.split_once('=').ok_or("--at espera TICK:SENSOR=VALOR")?;
                        roteiro.at(
                            tick.parse().map_err(|_| "tick deve ser inteiro")?,
                            sensor,
                            valor.parse().map_err(|_| "valor do sensor deve ser número")?,
                        );
                    }
                    "--permitir-sem-registro" => permitir_sem_registro = true,
                    "--falhar-ator" => {
                        roteiro.falhar_ator(&args.next().ok_or("--falhar-ator exige NOME")?)
                    }
                    "--fallback" => {
                        let kv = args.next().ok_or("--fallback exige PRIMARIO=ALTERNATIVO")?;
                        let (prim, alt) =
                            kv.split_once('=').ok_or("--fallback espera PRIMARIO=ALTERNATIVO")?;
                        roteiro.fallback(prim, alt);
                    }
                    "--registrar-ator" => {
                        roteiro.registrar_ator(&args.next().ok_or("--registrar-ator exige NOME")?)
                    }
                    "--fxp-mode" => {
                        fxp_mode = Some(args.next().ok_or("--fxp-mode exige simulado|real|hibrido")?)
                    }
                    "--fxp-config" => {
                        fxp_config = Some(PathBuf::from(args.next().ok_or("--fxp-config exige ARQUIVO")?))
                    }
                    _ if arquivo.is_none() => arquivo = Some(a),
                    _ => return Err(format!("argumento inesperado: {a}\n{USO}")),
                }
            }
            Ok(Comando::Run {
                arquivo: arquivo.ok_or(format!("run exige <arquivo.vl>\n{USO}"))?,
                ticks,
                real_ms,
                persist_dir,
                caderno,
                roteiro,
                permitir_sem_registro,
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
                    _ => return Err(format!("argumento inesperado: {a}\n{USO}")),
                }
            }
            Ok(Comando::FxpProbe { fxp_mode, fxp_config })
        }
        "caderno-verify" => {
            let mut arquivo = None;
            for a in args.by_ref() {
                if arquivo.is_none() {
                    arquivo = Some(a);
                } else {
                    return Err(format!("argumento inesperado: {a}\n{USO}"));
                }
            }
            Ok(Comando::CadernoVerify {
                arquivo: arquivo.ok_or(format!("caderno-verify exige <ARQUIVO>\n{USO}"))?,
            })
        }
        "--ajuda" | "-h" | "help" => Err(USO.to_string()),
        outra => Err(format!("subcomando desconhecido '{outra}'\n{USO}")),
    }
}

const USO: &str = "\
uso:
  vbl check <arquivo.vl> [--sem-registro]
  vbl run <arquivo.vl> [opções]
  vbl fxp-probe [--fxp-config ARQUIVO] [--fxp-mode MODO]
  vbl caderno-verify <ARQUIVO>

opções de run:
  --ticks N                        número de ticks virtuais (padrão: até esvaziar o mundo)
  --real-ms MS                     modo tempo real: 1 tick a cada MS milissegundos
  --persist-dir DIR                diretório de persistência `.vl` (padrão: persistencia/)
  --caderno ARQUIVO                Caderno de PRODUÇÃO (Etapa 4): gravação assíncrona;
                                   binário .vcad em ARQUIVO + JSONL em ARQUIVO.jsonl
  --set SENSOR=VALOR               valor inicial de sensor no FXP simulado
  --at TICK:SENSOR=VALOR           roteiriza valor absoluto de sensor no tick
  --falhar-ator NOME               ator para de responder (heartbeat — BDD Caso 3)
  --fallback PRIM=ALT              política de fallback do registro (FORMAL §4.3)
  --registrar-ator NOME            ator extra 0..255 safety 200 (ex.: VentoinhaReserva)
  --permitir-sem-registro          executa mesmo com referências fora do registro (§4.7)
  --fxp-mode MODO                  simulado|real|hibrido (padrão: simulado; sobrepõe a config)
  --fxp-config ARQUIVO             registro/config FXP (dispositivos, endpoints, fallback)

opções de fxp-probe:
  --fxp-config ARQUIVO             registro/config FXP a auditar
  --fxp-mode MODO                  simulado|real|hibrido (padrão: o da config, senão simulado)
";
