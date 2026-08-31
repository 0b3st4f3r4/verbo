//! Persistência de `equilibrium` (FORMAL §4.1): "na inicialização, o runtime
//! recarrega as `equilibrium` persistidas cujo `horizon` não venceu".
//!
//! O `.vl` canônico reparseável não carrega o instante de criação (a EBNF
//! não tem atributo para isso) — o instante vai em um sidecar JSON
//! (`<nome>.json` com `creation_time`). Sem sidecar, assume-se criação em
//! t=0 (época do programa). A recarga é auditada no Caderno (`recarga`).

use crate::caderno::{sha256_hex, Caderno};
use crate::engine::Engine;
use crate::fxp::Fxp;
use crate::json::Json;
use std::path::Path;
use vbl_lang::Conjugation;

/// Recarrega do diretório de persistência as `equilibrium` cujo horizon não
/// venceu. Usado na inicialização do runtime (antes de carregar o programa).
pub fn recarregar_equilibrium<F: Fxp>(engine: &mut Engine<F>) -> usize {
    let dir = engine.persistence_dir().to_path_buf();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0; // sem diretório: nada persistido ainda
    };
    let mut recarregadas = 0;
    for entrada in entries.flatten() {
        let caminho = entrada.path();
        if caminho.extension().and_then(|e| e.to_str()) != Some("vl") {
            continue;
        }
        let Ok(texto) = std::fs::read_to_string(&caminho) else {
            continue;
        };
        let (programa, diags) = vbl_lang::parse(&texto);
        if diags.has_errors() {
            engine.caderno.alert(
                &format!(
                    "Persistência inválida ignorada: {} — {} diagnósticos",
                    caminho.display(),
                    diags.errors().count()
                ),
                Json::obj([
                    ("motivo", Json::str("persistencia_invalida")),
                    ("caminho", Json::str(caminho.display().to_string())),
                ]),
            );
            continue;
        }
        for decl in programa.forms() {
            if decl.conjugation != Conjugation::Equilibrium {
                continue; // só `equilibrium` vive em suporte não volátil
            }
            // sidecar: instante de criação original (horizon absoluto)
            let sidecar = caminho.with_extension("json");
            let creation_time: f64 = std::fs::read_to_string(&sidecar)
                .ok()
                .and_then(|s| extrair_creation_time(&s))
                .unwrap_or(0.0);
            let mut form = crate::loader::forma_runtime(decl, creation_time);
            // horizon não venceu? (FORMAL §4.1)
            if form.horizon_esgotado(engine.sim_time) {
                continue;
            }
            // restaura cost_bytes efetivo: o gravado, se houver; senão o
            // tamanho real do arquivo (FORMAL §4.1)
            if form.cost_bytes.is_none() {
                form.cost_bytes = Some(std::fs::metadata(&caminho).map(|m| m.len()).unwrap_or(0));
            }
            let sha = sha256_hex(texto.as_bytes());
            engine.registrar_forma(form);
            engine.caderno.info(
                &format!(
                    "Forma '{}' recarregada do suporte estável ({}).",
                    decl.name,
                    caminho.display()
                ),
                Json::obj([
                    ("motivo", Json::str("recarga")),
                    ("forma", Json::str(&decl.name)),
                    ("caminho", Json::str(caminho.display().to_string())),
                    ("sha256", Json::str(&sha)),
                ]),
            );
            recarregadas += 1;
        }
    }
    recarregadas
}

/// Extrai `creation_time` do sidecar JSON mínimo.
fn extrair_creation_time(conteudo: &str) -> Option<f64> {
    let pos = conteudo.find("\"creation_time\"")?;
    let resto = &conteudo[pos + "\"creation_time\"".len()..];
    let pos_dois = resto.find(':')?;
    let numero: &str = resto[pos_dois + 1..]
        .trim()
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+' && c != 'e')
        .next()?;
    numero.trim().parse::<f64>().ok()
}

/// Grava o sidecar de metadados (`creation_time`) da forma persistida.
pub fn gravar_sidecar(dir: &Path, nome: &str, creation_time: f64) -> std::io::Result<()> {
    let conteudo = &format!("{{ \"creation_time\": {creation_time} }}\n");
    std::fs::write(dir.join(format!("{nome}.json")), conteudo)
}

