//! TLS do transporte remoto FXP v1.2/v1.3 (docs/FXP-SCHEMA-v1.md §7/§9).
//!
//! Confidencialidade e MAC **por frame**: o rustls (TLS 1.3) vive **sob** os
//! frames §2 — cada frame ≤ 8196 B viaja em um registro AEAD do TLS, então a
//! proteção do schema (princípio 4) sobe de "integridade do TCP" para
//! confidencialidade + autenticidade fim a fim do fluxo.
//!
//! **Escopo honesto da confiança:** rustls não expõe TLS-PSK (issue
//! rustls/rustls#174, aberta na v1.2), então a autenticação do servidor tem
//! dois modelos, ambos fail-closed (§4.6):
//!
//! * **Pinning (v1.2)** — certificado autoassinado + impressão digital
//!   SHA-256 do DER do certificado folha, fixada no endpoint do cliente
//!   (`tcps:host:porta@sha256:HEX`). Sem CA: você fala com quem o pin
//!   declarar, ou não fala.
//! * **TOFU (v1.3)** — `tcps:host:porta@tofu`: alternativa **operacional**
//!   ao pinning (o operador não copia o pin para o config do cliente). A
//!   impressão digital vista na PRIMEIRA conexão é gravada no store
//!   ([`TofuStore`]); as seguintes são verificadas contra ela; divergência ⇒
//!   handshake recusado. Honestidade: a primeira use ainda pode ser
//!   forjada — TOFU é mais fraco que pin e a diferença é documentada, não
//!   escondida.
//!
//! **Custo do handshake (v1.3):** o [`ClientConfig`] do cliente é cacheado
//! por chave de confiança — a retomada de sessão do rustls só acontece
//! quando a MESMA config é reusada — e o frame `CAPS` do handshake §4.5
//! pode sair como **0-RTT** na conexão retomada (sem PSK de aplicação, que
//! fala o servidor primeiro). Replay de 0-RTT é inofensivo aqui: `CAPS` é
//! idempotente por conexão (só inicia a negociação). Sem `--tls-*` no
//! servidor, o fio é byte a byte o da v1.1.
//!
//! Camadas independentes: AUTH PSK/CAPS (§4.5/§4.6) continuam existindo
//! **por cima** do TLS quando configuradas — TLS autentica o servidor por
//! pin/TOFU; PSK autentica o par na camada de aplicação. Unix não fala TLS
//! (o socket local já é confiado por acesso ao arquivo).

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Orçamento próprio do handshake TLS — pago 1× por conexão (o bus reutiliza
/// a conexão por endereço, §6); não consome o orçamento de leitura do fio.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// Teto do 0-RTT aceito pelo servidor (v1.3 §7): um frame `CAPS` cabe de
/// sobra; nada mais viaja antes do handshake completo.
pub(crate) const EARLY_DATA_MAX: u32 = 512;

/// Impressão digital SHA-256 do DER do certificado folha (`tcps:`).
pub type Fingerprint = [u8; 32];

/// Confiança declarada do endpoint `tcps:` (v1.3/v1.4 §7) — a única pergunta
/// feita ao certificado do servidor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// `@sha256:HEX` (v1.2) ou `@sha256:H1,H2,…` (v1.4 — rotação com
    /// sobreposição): o pin É a confiança declarada; QUALQUER um dos
    /// declarados autentica o servidor (o novo entra ANTES da troca do
    /// certificado, o velho sai DEPOIS).
    Pin(Vec<Fingerprint>),
    /// `@tofu` (v1.3): grava na primeira conexão, verifica nas seguintes.
    Tofu,
    /// `@tofu-estrito` (v1.4): exige entrada PRÉVIA no store
    /// ([`TofuStore`] como allow-list operacional) — nunca aprende sozinho;
    /// desconhecido ⇒ falha fechada com motivo `Desconhecida`. Aceita
    /// qualquer pin da entrada (rotação pela mesma porta da v1.3).
    TofuEstrito,
}

/// Confiança do cliente JÁ RESOLVIDA para a conexão (o TOFU carrega o store
/// aberto; o pin é autossuficiente) — parâmetro de [`client_stream`].
#[derive(Debug, Clone)]
pub enum ConfiancaCliente {
    /// Pin(s) fixo(s) (v1.2/v1.4 — qualquer um autentica).
    Pin(Vec<Fingerprint>),
    /// TOFU (v1.3): store compartilhado + alvo (`host:porta`).
    Tofu {
        store: Arc<Mutex<TofuStore>>,
        host: String,
        port: u16,
    },
    /// TOFU estrito (v1.4): o alvo DEVE estar no store (allow-list).
    TofuEstrito {
        store: Arc<Mutex<TofuStore>>,
        host: String,
        port: u16,
    },
}

/// Chave de cache da config cliente (v1.3 §7): a retomada de sessão exige o
/// MESMO `ClientConfig` entre conexões (o cache de tickets mora nele).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ChaveConfianca {
    Pin(Vec<Fingerprint>),
    Tofu {
        host: String,
        port: u16,
        store: PathBuf,
    },
    TofuEstrito {
        host: String,
        port: u16,
        store: PathBuf,
    },
}

impl From<&ConfiancaCliente> for ChaveConfianca {
    fn from(c: &ConfiancaCliente) -> Self {
        match c {
            ConfiancaCliente::Pin(fp) => ChaveConfianca::Pin(fp.clone()),
            ConfiancaCliente::Tofu { store, host, port } => ChaveConfianca::Tofu {
                host: host.clone(),
                port: *port,
                store: store.lock().map(|s| s.path.clone()).unwrap_or_default(),
            },
            ConfiancaCliente::TofuEstrito { store, host, port } => ChaveConfianca::TofuEstrito {
                host: host.clone(),
                port: *port,
                store: store.lock().map(|s| s.path.clone()).unwrap_or_default(),
            },
        }
    }
}

/// Config do lado servidor: cadeia + chave em PEM (arquivos do operador;
/// nunca segredos de env — o certificado é público, a chave é do disco).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TlsAccept {
    /// PEM com a cadeia do servidor (a folha primeiro).
    pub certs_pem: String,
    /// PEM da chave privada correspondente.
    pub key_pem: String,
    /// Cache de sessões TLS em DISCO (v1.4 §7 — sessão retomada entre
    /// renascimentos do PEER): o storage stateful do rustls persiste no
    /// arquivo (atômico, 0600 — blob de sessão é material de retomada).
    /// `None` ⇒ cache em memória (comportamento v1.3, por processo).
    /// Nota honesta: o lado CLIENTE do rustls 0.23 não expõe serialização
    /// de tickets (`Tls13ClientSessionValue` opaco — rustls#2287, resolvido
    /// só na 0.24), então o cache em disco é do lado do daemon.
    pub sessoes: Option<PathBuf>,
}

/// Falha de TLS — distinta de falha de schema/transporte puro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsError {
    /// PEM inválido, chave que não casa, config impossível.
    Config(String),
    /// Handshake não completou (timeout, pin divergente, TLS falado contra
    /// peer plano ou vice-versa).
    Handshake(String),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TlsError::Config(m) => write!(f, "config TLS inválida: {m}"),
            TlsError::Handshake(m) => write!(f, "handshake TLS falhou: {m}"),
        }
    }
}

impl std::error::Error for TlsError {}

/// Falha de TOFU (v1.3 §7; estrito v1.4) — distinta de falha de
/// config/handshake comum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TofuFalha {
    /// Impressão digital vista ≠ gravada na primeira conexão.
    Divergencia {
        armazenada: Fingerprint,
        vista: Fingerprint,
    },
    /// Modo ESTRITO (v1.4): o alvo não tem entrada no store — a confiança
    /// nasce declarada (allow-list operacional), nunca aprendida.
    Desconhecida { alvo: String },
    /// O store não pôde ser gravado — fail closed: permitir a conexão sem
    /// conseguir registrar a primeira use degradaria TOFU a "confiar em
    /// qualquer um", silenciosamente.
    Io(String),
}

impl std::fmt::Display for TofuFalha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TofuFalha::Divergencia { armazenada, vista } => write!(
                f,
                "impressão digital do servidor diverge da gravada na primeira conexão \
                 (armazenada sha256:{}, vista sha256:{})",
                hex32(armazenada),
                hex32(vista)
            ),
            TofuFalha::Desconhecida { alvo } => write!(
                f,
                "TOFU estrito: {alvo} não tem entrada no store (allow-list operacional — \
                 registre o pin antes da primeira conexão, v1.4 §7)"
            ),
            TofuFalha::Io(m) => write!(f, "store TOFU não pôde ser gravado: {m}"),
        }
    }
}

impl std::error::Error for TofuFalha {}

/// Store TOFU (v1.3 §7; multi-pin v1.4): `"host:porta" → [impressões
/// digitais]` em arquivo JSON (`Json` do runtime — determinístico, chaves
/// ordenadas). No modo aprendiz ([`Trust::Tofu`]) a primeira conexão GRAVA;
/// as seguintes VERIFICAM; no estrito ([`Trust::TofuEstrito`], v1.4) o alvo
/// DEVE já ter entrada (allow-list — nunca aprende). Entradas com MÚLTIPLAS
/// pins são a janela de rotação: qualquer um dos declarados autentica.
/// Divergência ou falha de persistência ⇒ falha fechada ([`TofuFalha`]).
#[derive(Debug)]
pub struct TofuStore {
    path: PathBuf,
    entradas: BTreeMap<String, Vec<Fingerprint>>,
}

impl TofuStore {
    /// Abre (ou inicia) o store em `path`. Arquivo corrompido ⇒ erro honesto
    /// (sem store confiável não há TOFU — nada de recriar silencioso).
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let entradas = match std::fs::read_to_string(path) {
            Ok(txt) => match json_para_store(&txt) {
                Ok(m) => m,
                Err(m) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, m)),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            path: path.to_path_buf(),
            entradas,
        })
    }

    /// Caminho padrão do store (estado do usuário): `$XDG_STATE_HOME` ou
    /// `$HOME/.local/state`, depois `verbo/fxp-known-hosts.json`. Sem HOME
    /// resolvível ⇒ `None` (o consumidor exige `--tofu-store`, honesto).
    pub fn caminho_padrao() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
        Some(base.join("verbo").join("fxp-known-hosts.json"))
    }

    /// Caminho no disco (chave de cache da config cliente).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Pins conhecidos do alvo (diagnóstico/CLI de rotação).
    pub fn pins_de(&self, alvo: &str) -> Option<&[Fingerprint]> {
        self.entradas.get(alvo).map(|v| v.as_slice())
    }

    /// Modo APRENDIZ (v1.3): verifica a impressão digital contra o store.
    /// Primeira use ⇒ grava (persistência atômica tmp+rename) e devolve
    /// `true`; conhecida e igual ⇒ `false`; divergente/impossível gravar ⇒
    /// falha.
    pub fn verificar(&mut self, alvo: &str, fp: Fingerprint) -> Result<bool, TofuFalha> {
        match self.entradas.get(alvo) {
            Some(pins) if pins.contains(&fp) => Ok(false),
            Some(pins) => Err(TofuFalha::Divergencia {
                armazenada: pins[0],
                vista: fp,
            }),
            None => {
                self.entradas.insert(alvo.to_string(), vec![fp]);
                self.persistir().map_err(|e| TofuFalha::Io(e.to_string()))?;
                Ok(true)
            }
        }
    }

    /// Modo ESTRITO (v1.4): o alvo DEVE ter entrada no store (allow-list
    /// operacional); qualquer pin da entrada autentica; desconhecido ⇒
    /// `Desconhecida` — o modo NUNCA aprende sozinho.
    pub fn verificar_estrito(&mut self, alvo: &str, fp: Fingerprint) -> Result<bool, TofuFalha> {
        match self.entradas.get(alvo) {
            Some(pins) if pins.contains(&fp) => Ok(false),
            Some(pins) => Err(TofuFalha::Divergencia {
                armazenada: pins[0],
                vista: fp,
            }),
            None => Err(TofuFalha::Desconhecida {
                alvo: alvo.to_string(),
            }),
        }
    }

    /// Rotação (v1.4 §7): adiciona um pin à entrada do alvo (o NOVO
    /// certificado entra ANTES do servidor trocar). Idempotente (`false`
    /// quando já presente, sem escrita). Persistência atômica.
    pub fn adicionar_pin(&mut self, alvo: &str, fp: Fingerprint) -> std::io::Result<bool> {
        let pins = self.entradas.entry(alvo.to_string()).or_default();
        if pins.contains(&fp) {
            return Ok(false);
        }
        pins.push(fp);
        self.persistir()?;
        Ok(true)
    }

    /// Rotação (v1.4 §7): remove um pin da entrada (o cert VELHO sai DEPOIS
    /// da troca). Entrada que esvazia é removida por inteiro. Idempotente.
    pub fn remover_pin(&mut self, alvo: &str, fp: Fingerprint) -> std::io::Result<bool> {
        let Some(pins) = self.entradas.get_mut(alvo) else {
            return Ok(false);
        };
        let Some(pos) = pins.iter().position(|p| *p == fp) else {
            return Ok(false);
        };
        pins.remove(pos);
        if pins.is_empty() {
            self.entradas.remove(alvo);
        }
        self.persistir()?;
        Ok(true)
    }

    /// Persistência atômica: escreve em `.tmp` e renomea por cima.
    fn persistir(&self) -> std::io::Result<()> {
        if let Some(pai) = self.path.parent() {
            std::fs::create_dir_all(pai)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, self.serializar())?;
        std::fs::rename(&tmp, &self.path)
    }

    /// JSON determinístico (chaves ordenadas pelo `BTreeMap`). Entrada com
    /// um só pin segue no formato v1.3 (`"sha256:<hex64>"` — arquivos
    /// antigos não mudam à toa); entrada multi-pin usa o objeto v1.4
    /// `{"pins":["sha256:…",…]}`. Valores autodescritivos: o algoritmo vai
    /// no arquivo (trocar de hash no futuro não ambiguiza entradas antigas).
    fn serializar(&self) -> String {
        let pin = |fp: &Fingerprint| vbl_runtime::json::Json::Str(format!("sha256:{}", hex32(fp)));
        vbl_runtime::json::Json::Obj(
            self.entradas
                .iter()
                .map(|(alvo, pins)| {
                    let valor = if pins.len() == 1 {
                        pin(&pins[0])
                    } else {
                        let lista: Vec<vbl_runtime::json::Json> = pins.iter().map(pin).collect();
                        debug_assert!(!lista.is_empty());
                        vbl_runtime::json::Json::Obj(
                            [("pins".to_string(), vbl_runtime::json::Json::Arr(lista))]
                                .into_iter()
                                .collect(),
                        )
                    };
                    (alvo.clone(), valor)
                })
                .collect(),
        )
        .serialize()
    }
}

/// Store JSON (`{"alvo":"sha256:hex64",…}` v1.3; `{"alvo":{"pins":[…]}}`
/// v1.4; mistos são aceitos) → mapa. Hex puro do formato inicial também é
/// aceito. Qualquer violação ⇒ erro com motivo (nunca lixo parcial).
fn json_para_store(txt: &str) -> Result<BTreeMap<String, Vec<Fingerprint>>, &'static str> {
    let parsed = vbl_runtime::json::Json::parse(txt).ok_or("JSON inválido")?;
    let vbl_runtime::json::Json::Obj(map) = parsed else {
        return Err("store TOFU não é um objeto JSON");
    };
    let mut out = BTreeMap::new();
    for (alvo, v) in map {
        let fp_de = |hex: &str| -> Result<Fingerprint, &'static str> {
            let hex = hex.strip_prefix("sha256:").unwrap_or(hex);
            unhex32(hex).ok_or("impressão digital do store não é sha256 hex de 64 dígitos")
        };
        match v {
            // Formato v1.3 (string única).
            vbl_runtime::json::Json::Str(hex) => {
                out.insert(alvo, vec![fp_de(&hex)?]);
            }
            // Formato v1.4 (objeto multi-pin).
            vbl_runtime::json::Json::Obj(campos) => {
                let Some(vbl_runtime::json::Json::Arr(pins)) = campos.get("pins") else {
                    return Err("entrada multi-pin do store TOFU sem campo \"pins\" (lista)");
                };
                if pins.is_empty() {
                    return Err("entrada multi-pin do store TOFU com lista vazia");
                }
                let mut fps = Vec::with_capacity(pins.len());
                for p in pins {
                    let vbl_runtime::json::Json::Str(hex) = p else {
                        return Err("pin do store TOFU não é string");
                    };
                    fps.push(fp_de(hex)?);
                }
                out.insert(alvo, fps);
            }
            _ => return Err("valor do store TOFU não é string nem objeto de pins"),
        }
    }
    Ok(out)
}

/// Impressão digital SHA-256 do DER do certificado (pin do cliente, TXT do
/// mDNS v1.2, "pronto" do `fxpd`).
/// Impressão digital do PRIMEIRO certificado de um PEM (v1.2 §4.10: TXT
/// `pin` do anúncio mDNS). PEM sem certificado ⇒ None (honesto).
pub fn fingerprint_pem(pem: &str) -> Option<Fingerprint> {
    let mut rd = std::io::BufReader::new(pem.as_bytes());
    let mut itens = rustls_pemfile::certs(&mut rd);
    let cert = itens.next()?.ok()?.into_owned();
    Some(fingerprint(&cert))
}

pub fn fingerprint(cert: &CertificateDer<'_>) -> Fingerprint {
    Sha256::digest(cert.as_ref()).into()
}

/// Hex minúsculo da impressão digital (formato canônico do endpoint).
pub fn hex32(bytes: &Fingerprint) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Decodifica 64 hex (case-insensitive) em impressão digital — `None` se o
/// texto não for hex ou não tiver 32 bytes.
pub fn unhex32(s: &str) -> Option<Fingerprint> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, par) in s.as_bytes().chunks(2).enumerate() {
        let hi = (par[0] as char).to_digit(16)?;
        let lo = (par[1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// `ServerConfig` TLS 1.3 a partir dos PEMs — validação de cert/chave no
/// momento da chamada (o `fxpd` falha honesto no arranque, não no 1º cliente).
/// v1.3 §7: aceita 0-RTT até [`EARLY_DATA_MAX`] bytes — o replay é inofensivo
/// porque o único frame adiantado é o `CAPS`, idempotente por conexão.
pub(crate) fn server_config(accept: &TlsAccept) -> Result<rustls::ServerConfig, TlsError> {
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut accept.certs_pem.as_bytes())
            .collect::<Result<_, _>>()
            .map_err(|e| TlsError::Config(format!("PEM de certificado: {e}")))?;
    if certs.is_empty() {
        return Err(TlsError::Config(
            "nenhum certificado no PEM (--tls-cert)".into(),
        ));
    }
    let key = rustls_pemfile::private_key(&mut accept.key_pem.as_bytes())
        .map_err(|e| TlsError::Config(format!("PEM de chave: {e}")))?
        .ok_or_else(|| TlsError::Config("chave privada ausente no PEM (--tls-key)".into()))?;
    let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|e| TlsError::Config(e.to_string()))?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|e| TlsError::Config(format!("certificado e chave não casam: {e}")))?;
    cfg.max_early_data_size = EARLY_DATA_MAX;
    if let Some(p) = &accept.sessoes {
        // v1.4 §7: storage stateful em DISCO — a sessão sobrevive ao
        // renascimento do processo do peer. Ticketer stateless NÃO é o
        // caminho: o rustls desliga early data (0-RTT) quando o servidor é
        // stateless (server/tls13.rs — early_data_configured exige storage
        // stateful); em disco mantemos os DOIS: retomada entre processos E
        // 0-RTT. Store corrupto ⇒ arranque falha honesto.
        let cache = crate::sessoes::CacheSessoesDisco::open(
            p,
            crate::sessoes::SESSOES_TETO,
        )
        .map_err(|e| {
            TlsError::Config(format!(
                "store de sessões TLS {}: {e}",
                p.display()
            ))
        })?;
        cfg.session_storage = Arc::new(cache);
    }
    Ok(cfg)
}

/// Cache de config cliente (v1.3 §7) — a retomada de sessão do rustls só
/// acontece quando o MESMO `ClientConfig` é reusado; sem cache, cada conexão
/// pagava o handshake completo (§9 da v1.2). Chave = confiança declarada.
fn cache_configs() -> &'static Mutex<BTreeMap<ChaveConfianca, Arc<rustls::ClientConfig>>> {
    static CACHE: OnceLock<Mutex<BTreeMap<ChaveConfianca, Arc<rustls::ClientConfig>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Config cliente em cache (retomável). Falha de lock envenenado ⇒ honesto.
pub(crate) fn client_config_cached(
    confianca: &ConfiancaCliente,
) -> Result<Arc<rustls::ClientConfig>, TlsError> {
    let chave = ChaveConfianca::from(confianca);
    let mut cache = cache_configs()
        .lock()
        .map_err(|_| TlsError::Config("cache de config TLS envenenado".into()))?;
    if let Some(cfg) = cache.get(&chave) {
        return Ok(cfg.clone());
    }
    let cfg = match confianca {
        ConfiancaCliente::Pin(pins) => client_config(pins.clone())?,
        ConfiancaCliente::Tofu { store, host, port } => {
            client_config_tofu(store.clone(), host.clone(), *port, false)?
        }
        ConfiancaCliente::TofuEstrito { store, host, port } => {
            client_config_tofu(store.clone(), host.clone(), *port, true)?
        }
    };
    let cfg = Arc::new(cfg);
    cache.insert(chave, cfg.clone());
    Ok(cfg)
}

/// `ClientConfig` com o verificador de **pin**: a única pergunta feita ao
/// certificado do servidor é "sua impressão digital é UMA das que declarei?"
/// (v1.4: múltiplos pins = janela de rotação com sobreposição).
/// v1.3 §7: early data habilitado — na conexão retomada o frame `CAPS` parte
/// como 0-RTT (o replay é inofensivo: CAPS idempotente por conexão).
pub(crate) fn client_config(pins: Vec<Fingerprint>) -> Result<rustls::ClientConfig, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let algs = provider.signature_verification_algorithms;
    let verifier = Arc::new(VerificadorPin { pins, algs });
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TlsError::Config(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    cfg.enable_early_data = true;
    Ok(cfg)
}

/// `ClientConfig` com o verificador **TOFU** (v1.3 §7; estrito v1.4): a
/// primeira conexão grava a impressão digital no store e as seguintes
/// verificam contra ela; no modo ESTRITO o alvo DEVE já ter entrada
/// (allow-list — nunca aprende).
pub(crate) fn client_config_tofu(
    store: Arc<Mutex<TofuStore>>,
    host: String,
    port: u16,
    estrito: bool,
) -> Result<rustls::ClientConfig, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let algs = provider.signature_verification_algorithms;
    let verifier = Arc::new(VerificadorTofu {
        store,
        alvo: format!("{host}:{port}"),
        estrito,
        algs,
    });
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TlsError::Config(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    cfg.enable_early_data = true;
    Ok(cfg)
}

/// Verificador por impressão digital: sem CA, sem nome de host — o pin É a
/// confiança declarada (v1.4: QUALQUER um dos pins declarados; a lista é a
/// janela de rotação com sobreposição). Assinaturas de handshake seguem
/// verificadas pelos algoritmos do provider (a MAC do TLS 1.3 protege o
/// canal).
#[derive(Debug)]
struct VerificadorPin {
    pins: Vec<Fingerprint>,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for VerificadorPin {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if self.pins.contains(&fingerprint(end_entity)) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::General(
                "impressão digital do certificado do servidor diverge dos pins do endpoint \
                 (tcps:...@sha256:HEX[,HEX2,…]) — conexão recusada (fail closed, v1.2/v1.4 §7)"
                    .into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// Verificador **TOFU** (v1.3 §7; estrito v1.4): a impressão digital vista é
/// verificada contra o store — aprendiz: grava na primeira use; estrito: o
/// alvo DEVE ter entrada (allow-list; desconhecido ⇒ handshake morto com
/// motivo `Desconhecida`). Divergência ⇒ handshake morto. Assinaturas de
/// handshake seguem verificadas pelos algoritmos do provider (a MAC do TLS
/// 1.3 protege o canal).
#[derive(Debug)]
struct VerificadorTofu {
    store: Arc<Mutex<TofuStore>>,
    alvo: String,
    estrito: bool,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for VerificadorTofu {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let fp = fingerprint(end_entity);
        let mut store = self
            .store
            .lock()
            .map_err(|_| Error::General("store TOFU envenenado — conexão recusada".into()))?;
        let resultado = if self.estrito {
            store.verificar_estrito(&self.alvo, fp)
        } else {
            store.verificar(&self.alvo, fp)
        };
        match resultado {
            Ok(_registrada) => Ok(ServerCertVerified::assertion()),
            Err(falha) => Err(Error::General(format!(
                "TOFU{} ({alvo}): {falha} — conexão recusada (fail closed, v1.3/v1.4 §7)",
                if self.estrito { " estrito" } else { "" },
                alvo = self.alvo
            ))),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// Orçamento do handshake no socket bruto (bloqueante durante o aperto de
/// mãos; quem chama reaplica o timeout de trabalho depois).
fn preparar_socket(sock: &TcpStream) -> std::io::Result<()> {
    sock.set_nonblocking(false)?;
    sock.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    sock.set_write_timeout(Some(HANDSHAKE_TIMEOUT))
}

/// Handshake do lado cliente com orçamento próprio — devolve o stream pronto
/// para servir os frames §2 e o sinal de que o frame adiantado (0-RTT, v1.3
/// §7) foi ENVIADO e ACEITO pelo servidor (`Some(false)` = early data não
/// coube/não foi retomada — o chamador segue o caminho normal de negociação).
pub(crate) fn client_stream(
    cfg: Arc<rustls::ClientConfig>,
    sock: TcpStream,
    server_name: ServerName<'static>,
    early: Option<&[u8]>,
) -> Result<
    (
        rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
        bool,
    ),
    TlsError,
> {
    preparar_socket(&sock).map_err(|e| TlsError::Handshake(format!("socket: {e}")))?;
    let mut conn = rustls::ClientConnection::new(cfg, server_name)
        .map_err(|e| TlsError::Handshake(e.to_string()))?;
    // v1.3 §7 — 0-RTT: o ClientHello já está montado em `new()`; o frame
    // adiantado entra na fila LOGO DEPOIS dele (mesmo voo). Sem sessão para
    // retomar, `bytes_left()` é 0 ⇒ não tenta (o CAPS segue no caminho
    // normal, dentro do handshake completo).
    let mut early_enviado = false;
    if let Some(dados) = early {
        if let Some(mut janela) = conn.early_data() {
            if janela.bytes_left() >= dados.len() {
                janela
                    .write_all(dados)
                    .map_err(|e| TlsError::Handshake(format!("0-RTT: {e}")))?;
                early_enviado = true;
            }
        }
    }
    let mut s = rustls::StreamOwned::new(conn, sock);
    while s.conn.is_handshaking() {
        s.conn
            .complete_io(&mut s.sock)
            .map_err(|e| TlsError::Handshake(format!("aperto de mãos: {e}")))?;
    }
    let confirmado = early_enviado && s.conn.is_early_data_accepted();
    Ok((s, confirmado))
}

/// Handshake do lado servidor (aceite do listener TCP).
pub(crate) fn server_stream(
    cfg: Arc<rustls::ServerConfig>,
    sock: TcpStream,
) -> Result<rustls::StreamOwned<rustls::ServerConnection, TcpStream>, TlsError> {
    preparar_socket(&sock).map_err(|e| TlsError::Handshake(format!("socket: {e}")))?;
    let conn =
        rustls::ServerConnection::new(cfg).map_err(|e| TlsError::Handshake(e.to_string()))?;
    let mut s = rustls::StreamOwned::new(conn, sock);
    while s.conn.is_handshaking() {
        s.conn
            .complete_io(&mut s.sock)
            .map_err(|e| TlsError::Handshake(format!("aperto de mãos: {e}")))?;
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip_e_rejeicoes() {
        let fp = [0xabu8; 32];
        let hex = hex32(&fp);
        assert_eq!(hex.len(), 64);
        assert_eq!(hex, "ab".repeat(32));
        assert_eq!(unhex32(&hex), Some(fp));
        assert_eq!(
            unhex32(&hex.to_uppercase()),
            Some(fp),
            "hex maiúsculo aceito"
        );
        assert_eq!(unhex32("ab"), None, "curto");
        assert_eq!(unhex32(&"zz".repeat(32)), None, "não-hex");
        assert_eq!(unhex32(&format!("{hex}00")), None, "longo");
    }

    #[test]
    fn fingerprint_e_deterministica_e_distingue_certs() {
        let a = rcgen::generate_simple_self_signed(vec!["a".into()]).expect("rcgen");
        let b = rcgen::generate_simple_self_signed(vec!["b".into()]).expect("rcgen");
        let fa = fingerprint(a.cert.der());
        let fb = fingerprint(b.cert.der());
        assert_eq!(fa, fingerprint(a.cert.der()), "mesmo DER ⇒ mesma impressão");
        assert_ne!(fa, fb, "certs distintos ⇒ impressões distintas");
    }

    #[test]
    fn server_config_valida_pem_e_falha_honesta() {
        let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("rcgen");
        let ok = TlsAccept {
            certs_pem: ck.cert.pem(),
            key_pem: ck.signing_key.serialize_pem(),
            sessoes: None,
        };
        assert!(server_config(&ok).is_ok());

        let sem_cert = TlsAccept {
            certs_pem: String::new(),
            key_pem: ok.key_pem.clone(),
            sessoes: None,
        };
        assert!(matches!(server_config(&sem_cert), Err(TlsError::Config(_))));

        let outra = rcgen::generate_simple_self_signed(vec!["outra".into()]).expect("rcgen");
        let chave_trocada = TlsAccept {
            certs_pem: ok.certs_pem.clone(),
            key_pem: outra.signing_key.serialize_pem(),
            sessoes: None,
        };
        assert!(matches!(
            server_config(&chave_trocada),
            Err(TlsError::Config(_))
        ));
    }
}

#[cfg(test)]
mod tests_v12_edge {
    use super::*;

    #[test]
    fn tls_error_display_honesto() {
        assert_eq!(
            TlsError::Config("pem ruim".into()).to_string(),
            "config TLS inválida: pem ruim"
        );
        assert_eq!(
            TlsError::Handshake("timeout".into()).to_string(),
            "handshake TLS falhou: timeout"
        );
    }

    #[test]
    fn fingerprint_pem_valido_e_garbage_honesto() {
        let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).expect("rcgen");
        let fp = fingerprint_pem(&ck.cert.pem()).expect("pem válido ⇒ fingerprint");
        assert_eq!(fp, fingerprint(ck.cert.der()));
        assert!(fingerprint_pem("isto não é um pem").is_none());
        assert!(fingerprint_pem("").is_none());
    }

    #[test]
    fn unhex32_recusa_tamanho_e_digito_errado() {
        assert!(unhex32("abc").is_none());
        assert!(unhex32(&"g".repeat(64)).is_none());
        assert!(unhex32(&"0".repeat(63)).is_none());
        let hex = hex32(&[7u8; 32]);
        assert_eq!(hex.len(), 64);
        assert_eq!(unhex32(&hex), Some([7u8; 32]));
    }
}
