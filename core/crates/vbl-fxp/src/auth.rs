//! Esquema de autenticação do canal remoto FXP v1.1 — PSK + HMAC-SHA256
//! (docs/FXP-SCHEMA-v1.md §4.6).
//!
//! **Escopo honesto:** autentica o **par** na abertura da conexão. Não cifra
//! e não autentica frames individuais — confidencialidade/MAC por frame
//! (rustls) é trabalho futuro registrado na §9. A integridade do fluxo segue
//! sendo a do Unix/TCP (princípio 4 do schema).
//!
//! - A chave **nunca** trafega; o que cruza o fio são nonces frescos por
//!   conexão e o HMAC `SHA-256(chave, "FXP-AUTH1" ‖ nonce_consumidor ‖
//!   nonce_servidor)` — nonces novos por conexão tornam a MAC não
//!   reutilizável (replay inútil).
//! - Verificação em tempo constante (`hmac::Mac::verify_slice`).

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Rótulo de domínio — separa esta MAC de qualquer outro uso da mesma chave.
pub const DOMAIN: &[u8] = b"FXP-AUTH1";

/// Tamanho do nonce/MAC (bytes) — espelha [`crate::schema::AUTH_NONCE_LEN`].
pub const NONCE_LEN: usize = 32;

/// Calcula a MAC de resposta do handshake (ambos os lados usam a mesma
/// composição; só quem tem a chave a reproduz).
pub fn mac(key: &[u8], nonce_consumidor: &[u8; NONCE_LEN], nonce_servidor: &[u8; NONCE_LEN]) -> [u8; NONCE_LEN] {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC aceita chave de qualquer tamanho");
    m.update(DOMAIN);
    m.update(nonce_consumidor);
    m.update(nonce_servidor);
    m.finalize().into_bytes().into()
}

/// Verificação em tempo constante da MAC apresentada pelo par.
pub fn verify(
    key: &[u8],
    nonce_consumidor: &[u8; NONCE_LEN],
    nonce_servidor: &[u8; NONCE_LEN],
    apresentada: &[u8; NONCE_LEN],
) -> bool {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC aceita chave de qualquer tamanho");
    m.update(DOMAIN);
    m.update(nonce_consumidor);
    m.update(nonce_servidor);
    m.verify_slice(apresentada).is_ok()
}

/// Nonce criptograficamente aleatório (OS RNG) — um par por conexão.
pub fn nonce() -> Result<[u8; NONCE_LEN], getrandom::Error> {
    let mut n = [0u8; NONCE_LEN];
    getrandom::fill(&mut n)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_e_deterministica_e_verificacao_e_constante() {
        let k = b"chave-de-teste";
        let nc = [1u8; NONCE_LEN];
        let ns = [2u8; NONCE_LEN];
        let m1 = mac(k, &nc, &ns);
        let m2 = mac(k, &nc, &ns);
        assert_eq!(m1, m2, "mesma chave/nonces ⇒ mesma MAC");
        assert!(verify(k, &nc, &ns, &m1));
        // chave errada
        assert!(!verify(b"outra", &nc, &ns, &m1));
        // nonce trocado (replay de outra conexão)
        let ns2 = [3u8; NONCE_LEN];
        assert!(!verify(k, &nc, &ns2, &m1));
        // MAC adulterada
        let mut ruim = m1;
        ruim[0] ^= 1;
        assert!(!verify(k, &nc, &ns, &ruim));
    }

    #[test]
    fn nonces_sao_unicos_na_pratica() {
        let a = nonce().unwrap();
        let b = nonce().unwrap();
        assert_ne!(a, b, "nonces do OS RNG não podem coincidir");
    }
}
