//! Argumentos de linha de comando do `vbl` (sem dependências externas).

use std::path::PathBuf;
use crate::roteiro::Roteiro;

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

opções de run:
  --ticks N                        número de ticks virtuais (padrão: até esvaziar o mundo)
  --real-ms MS                     modo tempo real: 1 tick a cada MS milissegundos
  --persist-dir DIR                diretório de persistência `.vl` (padrão: persistencia/)
  --caderno ARQUIVO                exporta o Caderno em JSONL ao final
  --set SENSOR=VALOR               valor inicial de sensor no FXP simulado
  --at TICK:SENSOR=VALOR           roteiriza valor absoluto de sensor no tick
  --permitir-sem-registration      executa mesmo com referências fora do registro (§4.7)
";
