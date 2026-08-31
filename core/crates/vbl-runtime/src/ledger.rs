//! O Caderno — sistema de auditoria termodinâmica (FORMAL §4/§6; AGENTS §1.4).
//!
//! Cada evento é encadeado por SHA-256
//! (`hash_n = SHA-256(hash_{n-1} || linha_n)`), formando cadeia à prova de
//! adulteração: qualquer edição retroativa quebra `verify_chain()`. A linha
//! canônica é `seq \x1f kind \x1f msg` seguida, quando há campos extras, de
//! `\x1f` + JSON (chaves ordenadas) — mesma composição do protótipo da
//! Etapa 1, permitindo auditoria externa a partir do JSONL exportado.
//!
//! A gravação em **memória** ([`ChainLedger`]) é a implementação de
//! referência (determinística, com os eventos consultáveis). O **Caderno de
//! produção** da Etapa 4 — gravação assíncrona em buffer, formato binário
//! compacto `.vcad`, cadeia incremental e agregados — vive em
//! [`crate::production_ledger`] (PLAN §4.1) e pluga pelo mesmo trait, sem
//! mudar o runtime.
//!
//! **Timestamp do relógio virtual (Etapa 4, AGENTS §1.4):** todo evento recebe
//! `tick` e `t` (segundos virtuais) injetados pelo engine via
//! [`Ledger::set_time`]. Os campos entram no `extra` do evento (chaves
//! reservadas `tick`/`t`), preservando a composição canônica da linha — a
//! cadeia continua verificável pelo mesmo método, e o JSONL exportado expõe os
//! timestamps no nível superior do objeto.

use crate::json::Json;
use sha2::{Digest, Sha256};

/// Um evento do Caderno.
#[derive(Debug, Clone)]
pub struct LedgerEvent {
    pub seq: usize,
    pub kind: String,
    pub msg: String,
    pub extra: Json,
    pub hash: String,
}

impl LedgerEvent {
    /// Linha canônica que entra na cadeia (sem o próprio hash). Pública para
    /// o Caderno de produção e o verificador externo (mesma composição).
    pub fn line(&self) -> String {
        let mut line = String::new();
        self.write_line(&mut line);
        line
    }

    /// Escreve a linha canônica diretamente em `out` (Etapa 5 — caminho
    /// quente do Caderno de produção reutiliza o buffer; sem alocação da
    /// string inteira a cada evento). Composição idêntica a [`LedgerEvent::line`].
    pub fn write_line(&self, out: &mut String) {
        use std::fmt::Write as _;
        let _ = write!(out, "{}\u{1f}{}\u{1f}{}", self.seq, self.kind, self.msg);
        if let Json::Obj(fields) = &self.extra {
            if !fields.is_empty() {
                out.push('\u{1f}');
                self.extra.serialize_into(out);
            }
        }
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
pub trait Ledger {
    fn record(&mut self, kind: &str, msg: &str, extra: Json);

    /// Define o relógio virtual corrente (chamado pelo engine a cada tick).
    /// Todo evento gravado depois carrega `tick` e `t` no `extra`
    /// (AGENTS §1.4: timestamp do relógio virtual em todos os eventos).
    fn set_time(&mut self, _tick: u64, _t: f64) {}

    /// Define a potência global do tick (W) — insumo do custo energético
    /// estimado das atuações registradas no mesmo tick (PLAN §4.1).
    fn set_power(&mut self, _watts: f64) {}

    /// Potência global corrente (W), se conhecida.
    fn current_power(&self) -> Option<f64> {
        None
    }

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

    fn collapse(&mut self, msg: &str, extra: Json) {
        self.record("COLAPSO", msg, extra);
    }

    fn art(&mut self, msg: &str, extra: Json) {
        self.record("SUBVERSAO", msg, extra);
    }

    /// Vazamento energético: potência partilhada × duração do tick (FORMAL §4.2).
    fn leak(&mut self, form: &str, watts: f64, seconds: f64) {
        let (msg, extra) = leak_event(form, watts, seconds);
        self.record("VAZAMENTO", &msg, extra);
    }

    fn sensor_read(&mut self, sensor: &str, value: f64) {
        self.record(
            "LEITURA",
            &format!("Sensor '{sensor}' = {value}"),
            Json::obj([("sensor", Json::str(sensor)), ("valor", Json::num(value))]),
        );
    }

    /// Atuação simples (compatibilidade): sem valor aplicado nem latência.
    fn actuator_action(&mut self, actor: &str, value: &crate::fxp::Value, success: bool) {
        self.actuator_action_detailed(Actuation {
            actor: actor.to_owned(),
            requested: value.clone(),
            applied: if success { Some(value.clone()) } else { None },
            latency_us: None,
            joule_cost: None,
            success,
        });
    }

    /// Atuação com a trilha completa da Etapa 4 (PLAN §4.1; FORMAL §4.3):
    /// ator, valor solicitado, valor aplicado, latência (µs) e custo
    /// energético da atuação. O custo, quando não informado, é estimado
    /// honestamente como potência do tick × latência do ack (J), marcado
    /// `custo_estimado_joules` no `extra`.
    fn actuator_action_detailed(&mut self, mut a: Actuation) {
        if let (Some(us), None) = (a.latency_us, a.joule_cost) {
            a.joule_cost = self.current_power().map(|w| w * us as f64 / 1e6);
        }
        let status = if a.success { "sucesso" } else { "falha" };
        let msg = match (&a.applied, a.latency_us) {
            (Some(applied), Some(us)) => format!(
                "Ator '{}' <- {} (aplicado: {applied}, {us} µs, {status})",
                a.actor, a.requested
            ),
            (Some(applied), None) => {
                format!("Ator '{}' <- {} (aplicado: {applied}, {status})", a.actor, a.requested)
            }
            (None, Some(us)) => {
                format!("Ator '{}' <- {} ({us} µs, {status})", a.actor, a.requested)
            }
            (None, None) => format!("Ator '{}' <- {} ({status})", a.actor, a.requested),
        };
        let mut fields = vec![
            ("ator", Json::str(&a.actor)),
            ("valor", a.requested.to_json()),
            ("sucesso", Json::boolean(a.success)),
        ];
        if let Some(applied) = &a.applied {
            fields.push(("aplicado", applied.to_json()));
        }
        if let Some(us) = a.latency_us {
            fields.push(("latencia_us", Json::num(us as f64)));
        }
        if let Some(j) = a.joule_cost {
            fields.push(("custo_estimado_joules", Json::num(j)));
        }
        self.record("ATUACAO", &msg, Json::obj(fields));
    }
}

/// Registro completo de uma atuação (Etapa 4 — PLAN §4.1).
#[derive(Debug, Clone)]
pub struct Actuation {
    pub actor: String,
    /// Valor solicitado pela regra `act`.
    pub requested: crate::fxp::Value,
    /// Valor efetivamente aplicado pelo driver (quando houve entrega).
    pub applied: Option<crate::fxp::Value>,
    /// Latência do ack do driver, em microssegundos.
    pub latency_us: Option<u64>,
    /// Custo energético estimado da atuação: potência × latência (J).
    pub joule_cost: Option<f64>,
    pub success: bool,
}

/// Mensagem/extra canônicos do evento VAZAMENTO (compartilhado pelas
/// implementações do trait — FORMAL §4.2).
pub(crate) fn leak_event(form: &str, watts: f64, seconds: f64) -> (String, Json) {
    let joules = watts * seconds;
    (
        format!("Forma '{form}' dissipou {joules:.2} Joules ({watts:.2} W por {seconds:.2}s)"),
        Json::obj([
            ("forma", Json::str(form)),
            ("watts", Json::num(watts)),
            ("segundos", Json::num(seconds)),
            ("joules", Json::num(joules)),
        ]),
    )
}

/// Caderno nulo — referência do A/B de overhead (bench da Etapa 4): absorve
/// os eventos sem custo além do dispatch do trait. Etapa 5: `leak` também é
/// no-op — "logger DESLIGADO" não constrói evento algum (o default do trait
/// monta msg/extra antes de chamar `record`, o que inflaria o A/B com custo
/// de construção, não de logging).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopLedger;

impl Ledger for NoopLedger {
    fn record(&mut self, _kind: &str, _msg: &str, _extra: Json) {}
    fn leak(&mut self, _form: &str, _watts: f64, _seconds: f64) {}
}

/// Implementação de referência: eventos em memória + cadeia SHA-256.
#[derive(Debug, Clone)]
pub struct ChainLedger {
    pub events: Vec<LedgerEvent>,
    chain_head: String,
    /// Relógio virtual corrente (tick, segundos) — carimbado em cada evento.
    time_s: (u64, f64),
    /// Potência global do tick (W) — insumo do custo estimado de atuações.
    power: f64,
}

impl Default for ChainLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainLedger {
    /// Cabeça da cadeia inicial: 64 zeros.
    pub const INITIAL_HEAD: &'static str = "0000000000000000000000000000000000000000000000000000000000000000";

    pub fn new() -> Self {
        Self { events: Vec::new(), chain_head: Self::INITIAL_HEAD.to_owned(), time_s: (0, 0.0), power: 0.0 }
    }

    pub fn reset(&mut self) {
        self.events.clear();
        self.chain_head = Self::INITIAL_HEAD.to_owned();
        self.time_s = (0, 0.0);
        self.power = 0.0;
    }

    pub fn chain_head(&self) -> &str {
        &self.chain_head
    }

    /// Recomputa a cadeia a partir dos eventos e confere a cabeça.
    pub fn verify_chain(&self) -> bool {
        let mut head = Self::INITIAL_HEAD.to_owned();
        for e in &self.events {
            head = sha256_double_hex(head.as_bytes(), e.line().as_bytes());
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
        for e in &self.events {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert("seq".to_string(), Json::num(e.seq as f64));
            fields.insert("kind".to_string(), Json::str(&e.kind));
            fields.insert("msg".to_string(), Json::str(&e.msg));
            if let Json::Obj(extra) = &e.extra {
                for (k, v) in extra {
                    fields.insert(k.clone(), v.clone());
                }
            }
            fields.insert("hash".to_string(), Json::str(&e.hash));
            let line = Json::Obj(fields).serialize();
            writeln!(f, "{line}")?;
        }
        f.flush()?;
        Ok(self.events.len())
    }

    // ------------------------------------------------------------------
    // Consultas (espelham ConsultaCaderno da suíte Python)
    // ------------------------------------------------------------------
    pub fn kinds(&self) -> Vec<&str> {
        self.events.iter().map(|e| e.kind.as_str()).collect()
    }

    pub fn has(&self, kind: &str) -> bool {
        self.events.iter().any(|e| e.kind == kind)
    }

    /// Eventos de um kind, com filtro opcional por campos extras.
    pub fn search(&self, kind: &str, filter: &[(&str, Json)]) -> Vec<&LedgerEvent> {
        self.events
            .iter()
            .filter(|e| e.kind == kind && filter.iter().all(|(k, v)| field_equal(&e.extra, k, v)))
            .collect()
    }

    pub fn count_with(&self, kind: &str, filter: &[(&str, Json)]) -> bool {
        !self.search(kind, filter).is_empty()
    }
}

fn field_equal(extra: &Json, key: &str, expected: &Json) -> bool {
    match extra {
        Json::Obj(fields) => fields.get(key) == Some(expected),
        _ => false,
    }
}

/// SHA-256 em hex minúsculo.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex_minusculo(&h.finalize())
}

/// SHA-256 incremental sobre DUAS fatias (`a || b`) em hex — mesmo digest de
/// `sha256_hex([a b] concatenados)`, sem alocar a concatenação (Etapa 5: o
/// elo da cadeia é `SHA-256(head || line)` e ambos já existem separados).
pub fn sha256_double_hex(a: &[u8], b: &[u8]) -> String {
    hex_minusculo(&sha256_double_digest(a, b))
}

/// Digest cru (32 bytes) sobre duas fatias — o frame `.vcad` grava o elo em
/// bytes crus; hex só quando o destino é texto (Etapa 5: elimina a ida e
/// volta hex → cru da Etapa 4).
pub fn sha256_double_bytes(a: &[u8], b: &[u8]) -> [u8; 32] {
    sha256_double_digest(a, b)
}

fn sha256_double_digest(a: &[u8], b: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(a);
    h.update(b);
    h.finalize().into()
}

/// Tabela hex (Etapa 5 — substitui `format!("{b:02x}")` por byte).
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Escreve `data` em hex minúsculo no buffer (sem alocação intermediária).
pub fn write_hex(data: &[u8], out: &mut String) {
    out.reserve(data.len() * 2);
    for b in data {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
}

fn hex_minusculo(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    write_hex(data, &mut out);
    out
}

impl Ledger for ChainLedger {
    fn set_time(&mut self, tick: u64, t: f64) {
        self.time_s = (tick, t);
    }

    fn set_power(&mut self, watts: f64) {
        self.power = watts;
    }

    fn current_power(&self) -> Option<f64> {
        Some(self.power)
    }

    fn record(&mut self, kind: &str, msg: &str, mut extra: Json) {
        stamp_time(&mut extra, self.time_s.0, self.time_s.1);
        let seq = self.events.len();
        let mut event =
            LedgerEvent { seq, kind: kind.to_owned(), msg: msg.to_owned(), extra, hash: String::new() };
        let mut line = String::with_capacity(128);
        event.write_line(&mut line);
        event.hash = sha256_double_hex(self.chain_head.as_bytes(), line.as_bytes());
        self.chain_head = event.hash.clone();
        self.events.push(event);
    }
}

/// Injeta o relógio virtual no `extra` do evento (chaves reservadas `tick`
/// e `t`). Fica DENTRO do extra para preservar a composição canônica da
/// linha — a cadeia segue verificável pelo método da Etapa 1/2.
pub fn stamp_time(extra: &mut Json, tick: u64, t: f64) {
    if let Json::Obj(fields) = extra {
        fields.insert("tick".to_owned(), Json::num(tick as f64));
        fields.insert("t".to_owned(), Json::num(t));
    }
}
