//! Persistência de `equilibrium` (FORMAL §4.1): "na inicialização, o runtime
//! recarrega as `equilibrium` persistidas cujo `horizon` não venceu".
//!
//! O `.vl` canônico reparseável não carrega o instante de criação (a EBNF
//! não tem atributo para isso) — o instante vai em um sidecar JSON
//! (`<name>.json` com `creation_time`). Sem sidecar, assume-se criação em
//! t=0 (época do programa). A recarga é auditada no Caderno (`recarga`).

use crate::ledger::{sha256_hex, Ledger};
use crate::engine::Engine;
use crate::fxp::Fxp;
use crate::json::Json;
use std::path::Path;
use vbl_lang::Conjugation;

/// Recarrega do diretório de persistência as `equilibrium` cujo horizon não
/// venceu. Usado na inicialização do runtime (antes de carregar o programa).
pub fn reload_equilibrium<F: Fxp, C: Ledger>(engine: &mut Engine<F, C>) -> usize {
    let dir = engine.persistence_dir().to_path_buf();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0; // sem diretório: nada persistido ainda
    };
    let mut reloaded = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("vl") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (program, diags) = vbl_lang::parse(&text);
        if diags.has_errors() {
            engine.ledger.alert(
                &format!(
                    "Persistência inválida ignorada: {} — {} diagnósticos",
                    path.display(),
                    diags.errors().count()
                ),
                Json::obj([
                    ("motivo", Json::str("persistencia_invalida")),
                    ("caminho", Json::str(path.display().to_string())),
                ]),
            );
            continue;
        }
        for decl in program.forms() {
            if decl.conjugation != Conjugation::Equilibrium {
                continue; // só `equilibrium` vive em suporte não volátil
            }
            // sidecar: instante de criação original (horizon absoluto)
            let sidecar = path.with_extension("json");
            let creation_time: f64 = std::fs::read_to_string(&sidecar)
                .ok()
                .and_then(|s| extract_creation_time(&s))
                .unwrap_or(0.0);
            let mut form = crate::loader::runtime_form(decl, creation_time);
            // horizon não venceu? (FORMAL §4.1)
            if form.horizon_exhausted(engine.sim_time) {
                continue;
            }
            // restaura cost_bytes efetivo: o gravado, se houver; senão o
            // tamanho real do arquivo (FORMAL §4.1)
            if form.cost_bytes.is_none() {
                form.cost_bytes = Some(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0));
            }
            let sha = sha256_hex(text.as_bytes());
            engine.register_form(form);
            engine.ledger.info(
                &format!(
                    "Forma '{}' recarregada do suporte estável ({}).",
                    decl.name,
                    path.display()
                ),
                Json::obj([
                    ("motivo", Json::str("recarga")),
                    ("forma", Json::str(&decl.name)),
                    ("caminho", Json::str(path.display().to_string())),
                    ("sha256", Json::str(&sha)),
                ]),
            );
            reloaded += 1;
        }
    }
    reloaded
}

/// Extrai `creation_time` do sidecar JSON mínimo.
fn extract_creation_time(content: &str) -> Option<f64> {
    let pos = content.find("\"creation_time\"")?;
    let rest = &content[pos + "\"creation_time\"".len()..];
    let pos_dois = rest.find(':')?;
    let number: &str = rest[pos_dois + 1..]
        .trim()
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+' && c != 'e')
        .next()?;
    number.trim().parse::<f64>().ok()
}

/// Grava o sidecar de metadados (`creation_time`) da forma persistida.
pub fn write_sidecar(dir: &Path, name: &str, creation_time: f64) -> std::io::Result<()> {
    let content = &format!("{{ \"creation_time\": {creation_time} }}\n");
    std::fs::write(dir.join(format!("{name}.json")), content)
}

