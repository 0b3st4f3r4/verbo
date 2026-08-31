//! O Caderno — sistema de auditoria termodinâmica (FORMAL §4/§6; AGENTS §1.4).
//!
//! Cada evento é encadeado por SHA-256
//! (`hash_n = SHA-256(hash_{n-1} || linha_n)`), formando cadeia à prova de
//! adulteração: qualquer edição retroativa quebra `verify_chain()`. A linha
//! canônica é `seq \x1f kind \x1f msg` seguida, quando há campos extras, de
//! `\x1f` + JSON (chaves ordenadas) — mesma composição do protótipo da
//! Etapa 1, permitindo auditoria externa a partir do JSONL exportado.
//!
//! A gravação em **memória** ([`ChainCaderno`]) é a implementação de
//! referência (determinística, com os eventos consultáveis). O **Caderno de
//! produção** da Etapa 4 — gravação assíncrona em buffer, formato binário
//! compacto `.vcad`, cadeia incremental e agregados — vive em
//! [`crate::caderno_producao`] (PLAN §4.1) e pluga pelo mesmo trait, sem
//! mudar o runtime.
//!
//! **Timestamp do relógio virtual (Etapa 4, AGENTS §1.4):** todo evento recebe
//! `tick` e `t` (segundos virtuais) injetados pelo engine via
//! [`Caderno::definir_tempo`]. Os campos entram no `extra` do evento (chaves
//! reservadas `tick`/`t`), preservando a composição canônica da linha — a
//! cadeia continua verificável pelo mesmo método, e o JSONL exportado expõe os
//! timestamps no nível superior do objeto.

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
    /// Linha canônica que entra na cadeia (sem o próprio hash). Pública para
    /// o Caderno de produção e o verificador externo (mesma composição).
    pub fn linha(&self) -> String {
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

    /// Define o relógio virtual corrente (chamado pelo engine a cada tick).
    /// Todo evento gravado depois carrega `tick` e `t` no `extra`
    /// (AGENTS §1.4: timestamp do relógio virtual em todos os eventos).
    fn definir_tempo(&mut self, _tick: u64, _t: f64) {}

    /// Define a potência global do tick (W) — insumo do custo energético
    /// estimado das atuações registradas no mesmo tick (PLAN §4.1).
    fn definir_potencia(&mut self, _watts: f64) {}

    /// Potência global corrente (W), se conhecida.
    fn potencia_corrente(&self) -> Option<f64> {
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

    fn colapso(&mut self, msg: &str, extra: Json) {
        self.record("COLAPSO", msg, extra);
    }

    fn art(&mut self, msg: &str, extra: Json) {
        self.record("SUBVERSAO", msg, extra);
    }

    /// Vazamento energético: potência partilhada × duração do tick (FORMAL §4.2).
    fn leak(&mut self, forma: &str, watts: f64, segundos: f64) {
        let (msg, extra) = evento_vazamento(forma, watts, segundos);
        self.record("VAZAMENTO", &msg, extra);
    }

    fn sensor_read(&mut self, sensor: &str, valor: f64) {
        self.record(
            "LEITURA",
            &format!("Sensor '{sensor}' = {valor}"),
            Json::obj([("sensor", Json::str(sensor)), ("valor", Json::num(valor))]),
        );
    }

    /// Atuação simples (compatibilidade): sem valor aplicado nem latência.
    fn actuator_action(&mut self, ator: &str, valor: &crate::fxp::Value, sucesso: bool) {
        self.actuator_action_detalhada(Atuacao {
            ator: ator.to_owned(),
            solicitado: valor.clone(),
            aplicado: if sucesso { Some(valor.clone()) } else { None },
            latencia_us: None,
            custo_joules: None,
            sucesso,
        });
    }

    /// Atuação com a trilha completa da Etapa 4 (PLAN §4.1; FORMAL §4.3):
    /// ator, valor solicitado, valor aplicado, latência (µs) e custo
    /// energético da atuação. O custo, quando não informado, é estimado
    /// honestamente como potência do tick × latência do ack (J), marcado
    /// `custo_estimado_joules` no `extra`.
    fn actuator_action_detalhada(&mut self, mut a: Atuacao) {
        if let (Some(us), None) = (a.latencia_us, a.custo_joules) {
            a.custo_joules = self.potencia_corrente().map(|w| w * us as f64 / 1e6);
        }
        let status = if a.sucesso { "sucesso" } else { "falha" };
        let msg = match (&a.aplicado, a.latencia_us) {
            (Some(aplicado), Some(us)) => format!(
                "Ator '{}' <- {} (aplicado: {aplicado}, {us} µs, {status})",
                a.ator, a.solicitado
            ),
            (Some(aplicado), None) => {
                format!("Ator '{}' <- {} (aplicado: {aplicado}, {status})", a.ator, a.solicitado)
            }
            (None, Some(us)) => {
                format!("Ator '{}' <- {} ({us} µs, {status})", a.ator, a.solicitado)
            }
            (None, None) => format!("Ator '{}' <- {} ({status})", a.ator, a.solicitado),
        };
        let mut campos = vec![
            ("ator", Json::str(&a.ator)),
            ("valor", a.solicitado.to_json()),
            ("sucesso", Json::boolean(a.sucesso)),
        ];
        if let Some(aplicado) = &a.aplicado {
            campos.push(("aplicado", aplicado.to_json()));
        }
        if let Some(us) = a.latencia_us {
            campos.push(("latencia_us", Json::num(us as f64)));
        }
        if let Some(j) = a.custo_joules {
            campos.push(("custo_estimado_joules", Json::num(j)));
        }
        self.record("ATUACAO", &msg, Json::obj(campos));
    }
}

/// Registro completo de uma atuação (Etapa 4 — PLAN §4.1).
#[derive(Debug, Clone)]
pub struct Atuacao {
    pub ator: String,
    /// Valor solicitado pela regra `act`.
    pub solicitado: crate::fxp::Value,
    /// Valor efetivamente aplicado pelo driver (quando houve entrega).
    pub aplicado: Option<crate::fxp::Value>,
    /// Latência do ack do driver, em microssegundos.
    pub latencia_us: Option<u64>,
    /// Custo energético estimado da atuação: potência × latência (J).
    pub custo_joules: Option<f64>,
    pub sucesso: bool,
}

/// Mensagem/extra canônicos do evento VAZAMENTO (compartilhado pelas
/// implementações do trait — FORMAL §4.2).
pub(crate) fn evento_vazamento(forma: &str, watts: f64, segundos: f64) -> (String, Json) {
    let joules = watts * segundos;
    (
        format!("Forma '{forma}' dissipou {joules:.2} Joules ({watts:.2} W por {segundos:.2}s)"),
        Json::obj([
            ("forma", Json::str(forma)),
            ("watts", Json::num(watts)),
            ("segundos", Json::num(segundos)),
            ("joules", Json::num(joules)),
        ]),
    )
}

/// Caderno nulo — referência do A/B de overhead (bench da Etapa 4): absorve
/// os eventos sem custo além do dispatch do trait.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopCaderno;

impl Caderno for NoopCaderno {
    fn record(&mut self, _kind: &str, _msg: &str, _extra: Json) {}
}

/// Implementação de referência: eventos em memória + cadeia SHA-256.
#[derive(Debug, Clone)]
pub struct ChainCaderno {
    pub eventos: Vec<Evento>,
    chain_head: String,
    /// Relógio virtual corrente (tick, segundos) — carimbado em cada evento.
    tempo: (u64, f64),
    /// Potência global do tick (W) — insumo do custo estimado de atuações.
    potencia: f64,
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
        Self { eventos: Vec::new(), chain_head: Self::HEAD_INICIAL.to_owned(), tempo: (0, 0.0), potencia: 0.0 }
    }

    pub fn reset(&mut self) {
        self.eventos.clear();
        self.chain_head = Self::HEAD_INICIAL.to_owned();
        self.tempo = (0, 0.0);
        self.potencia = 0.0;
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
    fn definir_tempo(&mut self, tick: u64, t: f64) {
        self.tempo = (tick, t);
    }

    fn definir_potencia(&mut self, watts: f64) {
        self.potencia = watts;
    }

    fn potencia_corrente(&self) -> Option<f64> {
        Some(self.potencia)
    }

    fn record(&mut self, kind: &str, msg: &str, mut extra: Json) {
        carimbar_tempo(&mut extra, self.tempo.0, self.tempo.1);
        let seq = self.eventos.len();
        let mut evento =
            Evento { seq, kind: kind.to_owned(), msg: msg.to_owned(), extra, hash: String::new() };
        let linha = evento.linha();
        evento.hash = sha256_hex(format!("{}{linha}", self.chain_head).as_bytes());
        self.chain_head = evento.hash.clone();
        self.eventos.push(evento);
    }
}

/// Injeta o relógio virtual no `extra` do evento (chaves reservadas `tick`
/// e `t`). Fica DENTRO do extra para preservar a composição canônica da
/// linha — a cadeia segue verificável pelo método da Etapa 1/2.
pub fn carimbar_tempo(extra: &mut Json, tick: u64, t: f64) {
    if let Json::Obj(campos) = extra {
        campos.insert("tick".to_owned(), Json::num(tick as f64));
        campos.insert("t".to_owned(), Json::num(t));
    }
}
