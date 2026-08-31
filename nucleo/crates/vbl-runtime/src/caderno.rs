//! O Caderno — sistema de auditoria termodinâmica (FORMAL §4/§6; AGENTS §1.4).
//!
//! Cada evento é encadeado por SHA-256
//! (`hash_n = SHA-256(hash_{n-1} || linha_n)`), formando cadeia à prova de
//! adulteração: qualquer edição retroativa quebra `verify_chain()`. A linha
//! canônica é `seq \x1f kind \x1f msg` seguida, quando há campos extras, de
//! `\x1f` + JSON (chaves ordenadas) — mesma composição do protótipo da
//! Etapa 1, permitindo auditoria externa a partir do JSONL exportado.
//!
//! A gravação é **em memória + export JSONL** nesta etapa; o formato binário
//! compacto e a escrita assíncrona (buffer + flush, overhead ≤ 1%) são o
//! Caderno de produção da Etapa 4 (PLAN §4.1).

use crate::json::Json;
use sha2::{Digest, Sha256};

/// Um evento do Caderno.
#[derive(Debug, Clone)]
pub struct Evento {
    pub seq: usize,
    pub kind: String,
    pub msg: String,
    pub extra: Json,
    pub hash: String,
}

impl Evento {
    /// Linha canônica que entra na cadeia (sem o próprio hash).
    fn linha(&self) -> String {
        let mut linha = format!("{}\u{1f}{}\u{1f}{}", self.seq, self.kind, self.msg);
        if let Json::Obj(campos) = &self.extra {
            if !campos.is_empty() {
                linha.push('\u{1f}');
                linha.push_str(&self.extra.serializar());
            }
        }
        linha
    }
}

/// Kinds canônicos da FORMAL §6 (fins de forma e eventos de revisão).
pub mod kinds {
    pub const DISSOLVE_RULE: &str = "dissolve_rule";
    pub const DISSOLVE_HORIZON: &str = "dissolve_horizon";
    pub const COLLAPSE_MAINTENANCE: &str = "collapse_maintenance";
    pub const DISSOLVE_SUBVERT: &str = "dissolve_subvert";
    pub const REVIEW_SHORT_CIRCUIT: &str = "review_short_circuit";
    pub const REVIEW_AFTER_DISSOLUTION: &str = "review_after_dissolution";
    pub const ACTOR_REJECTED_VALUE: &str = "actor_rejected_value";
    pub const PERSISTENCIA: &str = "persistencia";
    pub const TRANSICAO: &str = "transicao";
    pub const SUBVERT_APLICADO: &str = "subvert_aplicado";
    pub const KEEP_FORMA_INEXISTENTE: &str = "keep_forma_inexistente";
    pub const KEEP_IGNORADO: &str = "keep_ignorado";
    pub const RECLASSIFY_SEM_DEADLINE: &str = "reclassify_sem_deadline";
    pub const ATOR_INEXISTENTE: &str = "ator_inexistente";
    pub const ATOR_INDISPONIVEL: &str = "ator_indisponivel";
    pub const FALLBACK_EXECUTADO: &str = "fallback_executado";
}

/// Interface do Caderno consumida pelo runtime e pelo FXP.
///
/// Um trait (não uma struct) para que o Caderno de produção da Etapa 4
/// (gravação assíncrona em buffer) plugue sem mudar o runtime.
pub trait Caderno {
    fn record(&mut self, kind: &str, msg: &str, extra: Json);

    // ------------------------------------------------------------------
    // Atalhos com os níveis canônicos do protótipo
    // ------------------------------------------------------------------
    fn info(&mut self, msg: &str, extra: Json) {
        self.record("INFO", msg, extra);
    }

    fn warn(&mut self, msg: &str, extra: Json) {
        self.record("AVALIACAO", msg, extra);
    }

    fn alert(&mut self, msg: &str, extra: Json) {
        self.record("ALERTA", msg, extra);
    }

    fn colapso(&mut self, msg: &str, extra: Json) {
        self.record("COLAPSO", msg, extra);
    }

    fn art(&mut self, msg: &str, extra: Json) {
        self.record("SUBVERSAO", msg, extra);
    }

    /// Vazamento energético: potência partilhada × duração do tick (FORMAL §4.2).
    fn leak(&mut self, forma: &str, watts: f64, segundos: f64) {
        let joules = watts * segundos;
        self.record(
            "VAZAMENTO",
            &format!(
                "Forma '{forma}' dissipou {joules:.2} Joules ({watts:.2} W por {segundos:.2}s)"
            ),
            Json::obj([
                ("forma", Json::str(forma)),
                ("watts", Json::num(watts)),
                ("segundos", Json::num(segundos)),
                ("joules", Json::num(joules)),
            ]),
        );
    }

    fn sensor_read(&mut self, sensor: &str, valor: f64) {
        self.record(
            "LEITURA",
            &format!("Sensor '{sensor}' = {valor}"),
            Json::obj([("sensor", Json::str(sensor)), ("valor", Json::num(valor))]),
        );
    }

    fn actuator_action(&mut self, ator: &str, valor: &crate::fxp::Value, sucesso: bool) {
        let status = if sucesso { "sucesso" } else { "falha" };
        self.record(
            "ATUACAO",
            &format!("Ator '{ator}' <- {valor} ({status})"),
            Json::obj([
                ("ator", Json::str(ator)),
                ("valor", valor.to_json()),
                ("sucesso", Json::boolean(sucesso)),
            ]),
        );
    }
}

/// Implementação de referência: eventos em memória + cadeia SHA-256.
#[derive(Debug, Clone)]
pub struct ChainCaderno {
    pub eventos: Vec<Evento>,
    chain_head: String,
}

impl Default for ChainCaderno {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainCaderno {
    /// Cabeça da cadeia inicial: 64 zeros.
    pub const HEAD_INICIAL: &'static str = "0000000000000000000000000000000000000000000000000000000000000000";

    pub fn new() -> Self {
        Self { eventos: Vec::new(), chain_head: Self::HEAD_INICIAL.to_owned() }
    }

    pub fn reset(&mut self) {
        self.eventos.clear();
        self.chain_head = Self::HEAD_INICIAL.to_owned();
    }

    pub fn chain_head(&self) -> &str {
        &self.chain_head
    }

    /// Recomputa a cadeia a partir dos eventos e confere a cabeça.
    pub fn verify_chain(&self) -> bool {
        let mut head = Self::HEAD_INICIAL.to_owned();
        for e in &self.eventos {
            head = sha256_hex(format!("{head}{}", e.linha()).as_bytes());
            if head != e.hash {
                return false;
            }
        }
        head == self.chain_head
    }

    /// Exporta o log JSONL (um evento por linha, com hash) para auditoria
    /// externa. Devolve o número de eventos gravados.
    pub fn export_jsonl(&self, path: &std::path::Path) -> std::io::Result<usize> {
        use std::io::Write;
        let mut f = std::fs::File::create(path)?;
        for e in &self.eventos {
            let mut campos = std::collections::BTreeMap::new();
            campos.insert("seq".to_string(), Json::num(e.seq as f64));
            campos.insert("kind".to_string(), Json::str(&e.kind));
            campos.insert("msg".to_string(), Json::str(&e.msg));
            if let Json::Obj(extra) = &e.extra {
                for (k, v) in extra {
                    campos.insert(k.clone(), v.clone());
                }
            }
            campos.insert("hash".to_string(), Json::str(&e.hash));
            let linha = Json::Obj(campos).serializar();
            writeln!(f, "{linha}")?;
        }
        f.flush()?;
        Ok(self.eventos.len())
    }

    // ------------------------------------------------------------------
    // Consultas (espelham ConsultaCaderno da suíte Python)
    // ------------------------------------------------------------------
    pub fn kinds(&self) -> Vec<&str> {
        self.eventos.iter().map(|e| e.kind.as_str()).collect()
    }

    pub fn tem(&self, kind: &str) -> bool {
        self.eventos.iter().any(|e| e.kind == kind)
    }

    /// Eventos de um kind, com filtro opcional por campos extras.
    pub fn buscar(&self, kind: &str, filtro: &[(&str, Json)]) -> Vec<&Evento> {
        self.eventos
            .iter()
            .filter(|e| e.kind == kind && filtro.iter().all(|(k, v)| campo_eq(&e.extra, k, v)))
            .collect()
    }

    pub fn tem_com(&self, kind: &str, filtro: &[(&str, Json)]) -> bool {
        !self.buscar(kind, filtro).is_empty()
    }
}

fn campo_eq(extra: &Json, chave: &str, esperado: &Json) -> bool {
    match extra {
        Json::Obj(campos) => campos.get(chave) == Some(esperado),
        _ => false,
    }
}

/// SHA-256 em hex minúsculo.
pub fn sha256_hex(dados: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(dados);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

impl Caderno for ChainCaderno {
    fn record(&mut self, kind: &str, msg: &str, extra: Json) {
        let seq = self.eventos.len();
        let mut evento =
            Evento { seq, kind: kind.to_owned(), msg: msg.to_owned(), extra, hash: String::new() };
        let linha = evento.linha();
        evento.hash = sha256_hex(format!("{}{linha}", self.chain_head).as_bytes());
        self.chain_head = evento.hash.clone();
        self.eventos.push(evento);
    }
}
