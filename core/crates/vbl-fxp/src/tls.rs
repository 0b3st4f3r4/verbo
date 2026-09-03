//! TLS do transporte remoto FXP v1.2 (docs/FXP-SCHEMA-v1.md §7/§9).
//!
//! Confidencialidade e MAC **por frame**: o rustls (TLS 1.3) vive **sob** os
//! frames §2 — cada frame ≤ 8196 B viaja em um registro AEAD do TLS, então a
//! proteção do schema (princípio 4) sobe de "integridade do TCP" para
//! confidencialidade + autenticidade fim a fim do fluxo.
//!
//! **Escopo honesto da confiança:** rustls não expõe TLS-PSK (issue
//! rustls/rustls#174, aberta na v1.2), então a autenticação do servidor é
//! **certificado autoassinado + impressão digital** — SHA-256 do DER do
//! certificado folha, fixada no endpoint do cliente
//! (`tcps:host:porta@sha256:HEX`). Sem CA, sem TOFU: você fala com quem o
//! pin declarar, ou não fala (fail closed — mesmo espírito do AUTH §4.6).
//! Sem `--tls-*` no servidor, o fio é byte a byte o da v1.1.
//!
//! Camadas independentes: AUTH PSK/CAPS (§4.5/§4.6) continuam existindo
//! **por cima** do TLS quando configuradas — TLS autentica o servidor por
//! pin; PSK autentica o par na camada de aplicação. Unix não fala TLS
//! (o socket local já é confiado por acesso ao arquivo).

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

/// Orçamento próprio do handshake TLS — pago 1× por conexão (o bus reutiliza
/// a conexão por endereço, §6); não consome o orçamento de leitura do fio.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// Impressão digital SHA-256 do DER do certificado folha (`tcps:`).
pub type Fingerprint = [u8; 32];

/// Config do lado servidor: cadeia + chave em PEM (arquivos do operador;
/// nunca segredos de env — o certificado é público, a chave é do disco).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TlsAccept {
    /// PEM com a cadeia do servidor (a folha primeiro).
    pub certs_pem: String,
    /// PEM da chave privada correspondente.
    pub key_pem: String,
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
pub(crate) fn server_config(accept: &TlsAccept) -> Result<rustls::ServerConfig, TlsError> {
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut accept.certs_pem.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|e| TlsError::Config(format!("PEM de certificado: {e}")))?;
    if certs.is_empty() {
        return Err(TlsError::Config("nenhum certificado no PEM (--tls-cert)".into()));
    }
    let key = rustls_pemfile::private_key(&mut accept.key_pem.as_bytes())
        .map_err(|e| TlsError::Config(format!("PEM de chave: {e}")))?
        .ok_or_else(|| TlsError::Config("chave privada ausente no PEM (--tls-key)".into()))?;
    rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TlsError::Config(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::Config(format!("certificado e chave não casam: {e}")))
}

/// `ClientConfig` com o verificador de **pin**: a única pergunta feita ao
/// certificado do servidor é "sua impressão digital é a que eu declarei?".
pub(crate) fn client_config(pin: Fingerprint) -> Result<rustls::ClientConfig, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let algs = provider.signature_verification_algorithms;
    let verifier = Arc::new(VerificadorPin { pin, algs });
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TlsError::Config(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

/// Verificador por impressão digital: sem CA, sem nome de host — o pin É a
/// confiança declarada. Assinaturas de handshake seguem verificadas pelos
/// algoritmos do provider (a MAC do TLS 1.3 protege o canal).
#[derive(Debug)]
struct VerificadorPin {
    pin: Fingerprint,
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
        if fingerprint(end_entity) == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::General(
                "impressão digital do certificado do servidor diverge do pin do endpoint \
                 (tcps:...@sha256:HEX) — conexão recusada (fail closed, v1.2 §7)"
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

/// Orçamento do handshake no socket bruto (bloqueante durante o aperto de
/// mãos; quem chama reaplica o timeout de trabalho depois).
fn preparar_socket(sock: &TcpStream) -> std::io::Result<()> {
    sock.set_nonblocking(false)?;
    sock.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    sock.set_write_timeout(Some(HANDSHAKE_TIMEOUT))
}

/// Handshake do lado cliente com orçamento próprio — devolve o stream pronto
/// para servir os frames §2.
pub(crate) fn client_stream(
    cfg: Arc<rustls::ClientConfig>,
    sock: TcpStream,
    server_name: ServerName<'static>,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, TlsError> {
    preparar_socket(&sock).map_err(|e| TlsError::Handshake(format!("socket: {e}")))?;
    let conn = rustls::ClientConnection::new(cfg, server_name)
        .map_err(|e| TlsError::Handshake(e.to_string()))?;
    let mut s = rustls::StreamOwned::new(conn, sock);
    while s.conn.is_handshaking() {
        s.conn
            .complete_io(&mut s.sock)
            .map_err(|e| TlsError::Handshake(format!("aperto de mãos: {e}")))?;
    }
    Ok(s)
}

/// Handshake do lado servidor (aceite do listener TCP).
pub(crate) fn server_stream(
    cfg: Arc<rustls::ServerConfig>,
    sock: TcpStream,
) -> Result<rustls::StreamOwned<rustls::ServerConnection, TcpStream>, TlsError> {
    preparar_socket(&sock).map_err(|e| TlsError::Handshake(format!("socket: {e}")))?;
    let conn = rustls::ServerConnection::new(cfg).map_err(|e| TlsError::Handshake(e.to_string()))?;
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
        assert_eq!(unhex32(&hex.to_uppercase()), Some(fp), "hex maiúsculo aceito");
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
        let ok = TlsAccept { certs_pem: ck.cert.pem(), key_pem: ck.signing_key.serialize_pem() };
        assert!(server_config(&ok).is_ok());

        let sem_cert = TlsAccept { certs_pem: String::new(), key_pem: ok.key_pem.clone() };
        assert!(matches!(server_config(&sem_cert), Err(TlsError::Config(_))));

        let outra = rcgen::generate_simple_self_signed(vec!["outra".into()]).expect("rcgen");
        let chave_trocada =
            TlsAccept { certs_pem: ok.certs_pem.clone(), key_pem: outra.signing_key.serialize_pem() };
        assert!(matches!(server_config(&chave_trocada), Err(TlsError::Config(_))));
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
