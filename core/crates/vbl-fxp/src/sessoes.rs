//! Cache de sessões TLS do servidor em DISCO (v1.4 §7) — a sessão retomada
//! entre RENASCIMENTOS do processo do peer.
//!
//! O que a v1.3 deixou em aberto (§9): a retomada de sessão dependia do
//! cache em memória — do lado do cliente, por `ClientConfig` (isso segue:
//! o rustls 0.23 não expõe serialização de tickets do cliente,
//! `Tls13ClientSessionValue` é opaco — rustls/rustls#2287, resolvido só na
//! 0.24 via PR #2907; quando a 0.24 estabilizar, o mesmo desenho se estende
//! ao cliente). Do lado do SERVIDOR, porém, o rustls 0.23 trafega o estado
//! de sessão como bytes crus (`StoresServerSessions`), e é aqui que o
//! ganho entre processos é real hoje: o `fxpd` que renasce (deploy, crash,
//! restart) recarrega os blobs e o cliente VIVO retoma a sessão — sem
//! pagar handshake completo, com 0-RTT intacto.
//!
//! **Por que storage stateful e não tickets stateless:** o rustls desliga
//! early data (0-RTT) quando o servidor usa `Ticketer` (server/tls13.rs —
//! `early_data_configured` exige storage stateful). Persistindo o storage
//! stateful em disco mantemos os DOIS ganhos: retomada entre processos E
//! 0-RTT.
//!
//! **Honestidade do material:** os blobs são estado de retomada TLS
//! (material de autenticação de sessão) — o arquivo é gravado com permissão
//! `0600` e vale a mesma advertência do store TOFU: quem lê o arquivo pode
//! retomar a sessão. Formato JSON determinístico
//! `{"sessoes":{"<chave hex>":{"blob":"<hex>","gravado_em":<epoch s>}}}`,
//! escrita atômica (`.tmp` + rename), evicção do mais velho acima do teto
//! e poda por idade (teto do TLS para tickets: 7 dias).

use std::collections::BTreeMap;
pub use rustls::server::StoresServerSessions;

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Teto padrão de entradas (mesma ordem do cache em memória do rustls —
/// `ServerSessionMemoryCache(1024)` do builder).
pub const SESSOES_TETO: usize = 1024;

/// Idade máxima de uma entrada em disco — acima disso a entrada é poda no
/// próximo `put`/abertura (o TLS 1.3 teta tickets em 7 dias; nada sobrevive
/// mais que isso).
const IDADE_MAX_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    blob: Vec<u8>,
    gravado_em: u64,
}

/// Cache de sessões do servidor com write-through em disco (v1.4 §7).
/// `Send + Sync` via `Mutex`; toda mutação persiste (frequência de
/// handshake — nunca no caminho de frame).
pub struct CacheSessoesDisco {
    path: std::path::PathBuf,
    teto: usize,
    entradas: Mutex<BTreeMap<Vec<u8>, Item>>,
}

impl std::fmt::Debug for CacheSessoesDisco {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheSessoesDisco")
            .field("path", &self.path)
            .field("teto", &self.teto)
            .finish_non_exhaustive()
    }
}

fn agora_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let pares: Vec<&[u8]> = s.as_bytes().chunks(2).collect();
    let mut out = Vec::with_capacity(pares.len());
    for par in pares {
        let hi = (par[0] as char).to_digit(16)?;
        let lo = (par[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

impl CacheSessoesDisco {
    /// Abre (ou inicia) o cache em `path`. Arquivo corrompido ⇒ erro honesto
    /// (o arranque do servidor falha — nunca recomeçar silencioso um store
    /// de material de sessão).
    pub fn open(path: &Path, teto: usize) -> std::io::Result<Self> {
        let entradas = match std::fs::read_to_string(path) {
            Ok(txt) => match json_para_sessoes(&txt) {
                Ok(m) => m,
                Err(m) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, m)),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            path: path.to_path_buf(),
            teto: teto.max(1),
            entradas: Mutex::new(entradas),
        })
    }

    /// Persistência atômica (`.tmp` + rename, `0600` — material de sessão).
    fn persistir(&self, entradas: &BTreeMap<Vec<u8>, Item>) -> std::io::Result<()> {
        use std::io::Write as _;
        if let Some(pai) = self.path.parent() {
            std::fs::create_dir_all(pai)?;
        }
        let itens: Vec<(String, vbl_runtime::json::Json)> = entradas
            .iter()
            .map(|(k, it)| {
                (
                    hex(k),
                    vbl_runtime::json::Json::obj([
                        ("blob", vbl_runtime::json::Json::Str(hex(&it.blob))),
                        (
                            "gravado_em",
                            vbl_runtime::json::Json::Num(it.gravado_em as f64),
                        ),
                    ]),
                )
            })
            .collect();
        let txt = vbl_runtime::json::Json::obj([(
            "sessoes",
            vbl_runtime::json::Json::Obj(itens.into_iter().collect()),
        )])
        .serialize();
        let tmp = self.path.with_extension("json.tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        // 0600 (unix): blob de sessão é material de retomada — quem lê o
        // arquivo pode retomar a sessão.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        f.write_all(txt.as_bytes())?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Poda por idade + evicção do mais velho acima do teto.
    fn podar(entradas: &mut BTreeMap<Vec<u8>, Item>, teto: usize) {
        let agora = agora_secs();
        entradas.retain(|_, it| agora.saturating_sub(it.gravado_em) < IDADE_MAX_SECS);
        while entradas.len() > teto {
            let mais_velho = entradas
                .iter()
                .min_by_key(|(_, it)| it.gravado_em)
                .map(|(k, _)| k.clone());
            match mais_velho {
                Some(k) => {
                    entradas.remove(&k);
                }
                None => break,
            }
        }
    }
}

impl StoresServerSessions for CacheSessoesDisco {
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> bool {
        let Ok(mut entradas) = self.entradas.lock() else {
            return false;
        };
        entradas.insert(
            key,
            Item {
                blob: value,
                gravado_em: agora_secs(),
            },
        );
        Self::podar(&mut entradas, self.teto);
        self.persistir(&entradas).is_ok()
    }

    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let entradas = self.entradas.lock().ok()?;
        entradas.get(key).map(|it| it.blob.clone())
    }

    fn take(&self, key: &[u8]) -> Option<Vec<u8>> {
        let mut entradas = self.entradas.lock().ok()?;
        let item = entradas.remove(key)?;
        // Sessão retomada é single-use no rustls: a remoção persiste.
        self.persistir(&entradas).ok()?;
        Some(item.blob)
    }

    fn can_cache(&self) -> bool {
        true
    }
}

/// JSON do store (`{"sessoes":{"<hex>":{"blob":"<hex>","gravado_em":N}}}`) →
/// mapa. Qualquer violação ⇒ erro com motivo (nunca lixo parcial).
fn json_para_sessoes(
    txt: &str,
) -> Result<BTreeMap<Vec<u8>, Item>, &'static str> {
    let parsed = vbl_runtime::json::Json::parse(txt).ok_or("JSON inválido")?;
    let vbl_runtime::json::Json::Obj(raiz) = parsed else {
        return Err("store de sessões não é um objeto JSON");
    };
    let Some(vbl_runtime::json::Json::Obj(map)) = raiz.get("sessoes") else {
        return Err("store de sessões sem o objeto \"sessoes\"");
    };
    let mut out = BTreeMap::new();
    for (k, v) in map {
        let chave = unhex(k).ok_or("chave do store de sessões não é hex")?;
        let vbl_runtime::json::Json::Obj(campos) = v else {
            return Err("entrada do store de sessões não é objeto");
        };
        let Some(vbl_runtime::json::Json::Str(blob_hex)) = campos.get("blob") else {
            return Err("entrada do store de sessões sem \"blob\" (string)");
        };
        let blob = unhex(blob_hex).ok_or("blob do store de sessões não é hex")?;
        let Some(vbl_runtime::json::Json::Num(gravado_em)) = campos.get("gravado_em") else {
            return Err("entrada do store de sessões sem \"gravado_em\" (número)");
        };
        if *gravado_em < 0.0 {
            return Err("\"gravado_em\" negativo");
        }
        out.insert(
            chave,
            Item {
                blob,
                gravado_em: *gravado_em as u64,
            },
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let b = vec![0x00, 0xff, 0x10];
        assert_eq!(unhex(&hex(&b)), Some(b.clone()));
        assert_eq!(unhex("abc"), None, "ímpar");
        assert_eq!(unhex("zz"), None, "não-hex");
    }
}
