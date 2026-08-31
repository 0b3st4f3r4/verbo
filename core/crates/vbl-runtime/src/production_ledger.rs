//! Caderno de produção (Etapa 4 — PLAN §4.1; AGENTS §1.4).
//!
//! [`ProductionLedger`] é a implementação do trait [`Ledger`] para execuções
//! reais: gravação **assíncrona em buffer** (thread dedicada + canal; o loop
//! de tick só serializa e enfileira — PLAN §4.3 "overhead do Caderno pode
//! distorcer medições"), **formato binário compacto** `.vcad` (mesma filosofia
//! zero-dependência do schema FXP v1; Cap'n Proto/FlatBuffers citados no
//! AGENTS são exemplos, não exigência) e **agregados** para monitoramento.
//!
//! Honestidade: a thread de gravação NUNCA inventa eventos; a cadeia SHA-256
//! é incremental (só a cabeça fica em memória) e pode ser reavermelhada por
//! um agente externo com `vbl ledger-verify` (binário ou JSONL exportado).
//!
//! Formato `.vcad` v1 (spec: docs/NOTEBOOK-FORMAT-v1.md):
//!
//! ```text
//! header : "VCAD" | versao u8
//! frame* : [u32 LE len][linha UTF-8 len bytes][hash SHA-256 — 32 bytes crus]
//! footer : "VFIM" | eventos u32 LE | chain_head 64 bytes ASCII (tamanho fixo 72)
//! ```
//!
//! `line` é a MESMA linha canônica da cadeia (`seq ␟ kind ␟ msg [␟ extra_json]`
//! — ver [`crate::ledger::LedgerEvent::line`]); o hash do frame é o elo da cadeia
//! em bytes crus. O verificador recomputa `hash_n = SHA-256(hash_{n-1} ||
//! linha_n)` — adulteração retroativa quebra a cadeia.

use crate::ledger::{stamp_time, sha256_double_hex, Ledger, ChainLedger};
use crate::json::{write_number, write_string, Json};
use std::collections::BTreeMap;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

/// Magic do formato binário.
pub const MAGIC: &[u8; 4] = b"VCAD";
/// Magic do rodapé (tamanho fixo — leitura determinística pelo fim).
pub const FOOTER_MAGIC: &[u8; 4] = b"VFIM";
/// Versão do formato.
pub const VERSION: u8 = 1;
/// Rodapé: magic (4) + eventos u32 (4) + chain_head 64 ASCII.
pub const FOOTER_BYTES: usize = 4 + 4 + 64;
/// Flush periódico: a cada N eventos (buffer + flush — PLAN §4.3).
const FLUSH_EVERY: usize = 256;
/// Tamanho do buffer de escrita.
const BUFFER_BYTES: usize = 64 * 1024;

/// Mensagem tick-thread → thread de gravação. O carimbo de tempo viaja
/// solto: a thread de gravação injeta no `extra` antes de compor a linha.
///
/// Etapa 5: o caminho quente ([`Ledger::leak`] — um evento por forma por
/// tick) viaja como dados crus ([`Msg::Leak`]); a linha canônica é
/// composta na thread de gravação diretamente no buffer reutilizado, sem
/// `Json`/`BTreeMap` intermediários — bytes idênticos à composição geral
/// (garantido por teste de equivalência).
enum Msg {
    LedgerEvent { seq: usize, tick: u64, t: f64, kind: String, msg: String, extra: Json },
    Leak(Leak),
    End,
}

/// Dados crus do evento LEAK (v1: VAZAMENTO) (caminho quente da Etapa 5) — a linha
/// canônica só existe na thread de gravação, direto no buffer reutilizado.
struct Leak {
    seq: usize,
    tick: u64,
    t: f64,
    form: String,
    watts: f64,
    seconds: f64,
    joules: f64,
}

/// Agregados do Caderno de produção (expostos para monitoramento —
/// AGENTS §1.4: métricas agregadas, Joules totais e médias).
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub events: usize,
    pub bytes: u64,
    pub chain_head: String,
    pub total_joules: f64,
    pub joules_per_form: BTreeMap<String, f64>,
    pub counts: BTreeMap<String, u64>,
}

impl Summary {
    /// Média de Joules por forma com vazamento registrado.
    pub fn avg_joules_per_form(&self) -> f64 {
        if self.joules_per_form.is_empty() {
            0.0
        } else {
            self.total_joules / self.joules_per_form.len() as f64
        }
    }
}

/// Caderno de produção: eventos fluem por canal para uma thread de gravação
/// (BufWriter + flush periódico); em memória restam apenas a sequência, o
/// relógio, a potência corrente e os agregados — memória limitada mesmo com
/// 10.000 formas ativas (AGENTS §1.4).
#[derive(Debug)]
pub struct ProductionLedger {
    tx: Option<Sender<Msg>>,
    handle: Option<JoinHandle<std::io::Result<Summary>>>,
    path: PathBuf,
    seq: usize,
    enqueued: usize,
    time_s: (u64, f64),
    power: f64,
    total_joules: f64,
    joules_per_form: BTreeMap<String, f64>,
    counts: BTreeMap<String, u64>,
}

impl ProductionLedger {
    /// Abre o arquivo binário (`.vcad`) e inicia a thread de gravação.
    pub fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let (tx, rec) = channel::<Msg>();
        let thread_path = path.clone();
        let handle = std::thread::Builder::new()
            .name("caderno".into())
            .spawn(move || write_thread(rec, thread_path))?;
        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
            path,
            seq: 0,
            enqueued: 0,
            time_s: (0, 0.0),
            power: 0.0,
            total_joules: 0.0,
            joules_per_form: BTreeMap::new(),
            counts: BTreeMap::new(),
        })
    }

    /// Caminho do arquivo binário em gravação.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Eventos enfileirados até agora (gravados ou na fila).
    pub fn enqueued(&self) -> usize {
        self.enqueued
    }

    /// Encerra: sinaliza o fim, espera a thread drenar a fila e devolve os
    /// agregados. Idempotente via consumo de `self`.
    pub fn close(mut self) -> Result<Summary, String> {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::End); // erro ⇒ thread já morreu (relatado abaixo)
        }
        let write_summary = self
            .handle
            .take()
            .expect("handle presente até fechar")
            .join()
            .map_err(|_| "thread de gravação do Caderno panicou".to_string())?
            .map_err(|e| format!("gravação do Caderno ({}): {e}", self.path.display()))?;
        debug_assert_eq!(write_summary.events, self.enqueued);
        Ok(Summary {
            events: write_summary.events,
            bytes: write_summary.bytes,
            chain_head: write_summary.chain_head,
            total_joules: self.total_joules,
            joules_per_form: std::mem::take(&mut self.joules_per_form),
            counts: std::mem::take(&mut self.counts),
        })
    }
}

impl Drop for ProductionLedger {
    fn drop(&mut self) {
        // fechar() explícito é o caminho normal; o Drop garante que a thread
        // termina e o arquivo fica válido mesmo em caminhos de erro (sem
        // vazar thread nem arquivo truncado — AGENTS §1.3).
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::End);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Ledger for ProductionLedger {
    fn set_time(&mut self, tick: u64, t: f64) {
        self.time_s = (tick, t);
    }

    fn set_power(&mut self, watts: f64) {
        self.power = watts;
    }

    fn current_power(&self) -> Option<f64> {
        Some(self.power)
    }

    fn record(&mut self, kind: &str, msg: &str, extra: Json) {
        // caminho geral (eventos raros: transições, alertas…) — acumula sem
        // alocar a chave quando o kind já está na tabela (Etapa 5)
        match self.counts.get_mut(kind) {
            Some(c) => *c += 1,
            None => {
                self.counts.insert(kind.to_owned(), 1);
            }
        }
        let seq = self.seq;
        self.seq += 1;
        self.enqueued += 1;
        if let Some(tx) = &self.tx {
            // unbounded: a thread grava em buffer de 64 KiB; sobrecarga de
            // disco aparece como memória na fila, nunca como perda de evento
            // (cobertura de eventos 100% — AGENTS §1.4).
            let _ = tx.send(Msg::LedgerEvent {
                seq,
                tick: self.time_s.0,
                t: self.time_s.1,
                kind: kind.to_owned(),
                msg: msg.to_owned(),
                extra,
            });
        }
    }

    /// Caminho quente (Etapa 5 — PLAN §5.2): acumula os Joules e envia os
    /// dados crus; a linha canônica é composta na thread de gravação sem
    /// `format!`+`Json::obj` intermediários (o custo dominante medido na
    /// Etapa 4 — docs/STAGE-4-REPORT.md §3). Agregados e contagens têm o
    /// MESMO contrato do caminho geral.
    fn leak(&mut self, form: &str, watts: f64, seconds: f64) {
        let joules = watts * seconds;
        self.total_joules += joules;
        match self.joules_per_form.get_mut(form) {
            Some(j) => *j += joules,
            None => {
                self.joules_per_form.insert(form.to_owned(), joules);
            }
        }
        match self.counts.get_mut("LEAK") {
            Some(c) => *c += 1,
            None => {
                self.counts.insert("LEAK".to_owned(), 1);
            }
        }
        let seq = self.seq;
        self.seq += 1;
        self.enqueued += 1;
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Leak(Leak {
                seq,
                tick: self.time_s.0,
                t: self.time_s.1,
                form: form.to_owned(),
                watts,
                seconds,
                joules,
            }));
        }
    }
}

/// Corpo da thread de gravação: header, frames, footer.
///
/// Etapa 5: linha e cabeça da cadeia vivem em buffers REUTILIZADOS (um único
/// `String` de linha para todo o log) e o elo é calculado sobre duas fatias
/// com digest cru — sem concatenar `head+line` nem fazer a viagem hex → cru
/// de cada frame (docs/STAGE-4-REPORT.md §3: encoding direto).
fn write_thread(rec: Receiver<Msg>, path: PathBuf) -> std::io::Result<Summary> {
    let arquivo = std::fs::File::create(&path)?;
    let mut w = BufWriter::with_capacity(BUFFER_BYTES, arquivo);
    w.write_all(MAGIC)?;
    w.write_all(&[VERSION])?;

    let mut head = ChainLedger::INITIAL_HEAD.to_owned();
    let mut line = String::with_capacity(192);
    let mut events = 0usize;
    let mut bytes = 0u64;
    while let Ok(msg) = rec.recv() {
        match msg {
            Msg::End => break,
            Msg::LedgerEvent { seq, tick, t, kind, msg, mut extra } => {
                stamp_time(&mut extra, tick, t);
                let event =
                    crate::ledger::LedgerEvent { seq, kind, msg, extra, hash: String::new() };
                line.clear();
                event.write_line(&mut line);
            }
            Msg::Leak(v) => {
                line.clear();
                write_leak_line(&mut line, &v);
            }
        }
        let digest = crate::ledger::sha256_double_bytes(head.as_bytes(), line.as_bytes());
        w.write_all(&(line.len() as u32).to_le_bytes())?;
        w.write_all(line.as_bytes())?;
        w.write_all(&digest)?;
        head.clear();
        crate::ledger::write_hex(&digest, &mut head);
        events += 1;
        bytes += 4 + line.len() as u64 + 32;
        if events.is_multiple_of(FLUSH_EVERY) {
            w.flush()?;
        }
    }
    w.write_all(FOOTER_MAGIC)?;
    w.write_all(&(events as u32).to_le_bytes())?;
    w.write_all(head.as_bytes())?; // 64 bytes hex
    bytes += FOOTER_BYTES as u64;
    w.flush()?;
    Ok(Summary { events, bytes, chain_head: head, ..Default::default() })
}

/// Composição direta da linha canônica do evento LEAK (v1: VAZAMENTO) — byte a byte
/// idêntica a `leak_event` + `stamp_time` + `LedgerEvent::line`:
/// `seq ␟ LEAK ␟ msg ␟ {"form","joules","seconds","t","tick","watts"}`
/// (chaves em ordem de classificação; números no formato canônico do `Json`).
/// Equivalência garantida por teste (`tests/production_ledger.rs`).
fn write_leak_line(line: &mut String, v: &Leak) {
    use std::fmt::Write as _;
    let _ = write!(
        line,
        "{seq}\u{1f}LEAK\u{1f}Forma '{form}' dissipou {joules:.2} Joules ({watts:.2} W por {seconds:.2}s)\u{1f}",
        seq = v.seq,
        form = v.form,
        joules = v.joules,
        watts = v.watts,
        seconds = v.seconds,
    );
    line.push_str("{\"forma\":");
    write_string(&v.form, line);
    line.push_str(",\"joules\":");
    write_number(v.joules, line);
    line.push_str(",\"segundos\":");
    write_number(v.seconds, line);
    line.push_str(",\"t\":");
    write_number(v.t, line);
    line.push_str(",\"tick\":");
    write_number(v.tick as f64, line);
    line.push_str(",\"watts\":");
    write_number(v.watts, line);
    line.push('}');
}

// ======================================================================
// Verificação externa (agente externo — AGENTS §1.4: checksums SHA-256)
// ======================================================================

/// Relatório da verificação de um log do Caderno (binário ou JSONL).
#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub events: usize,
    pub chain_ok: bool,
    /// Primeiro evento cujo hash não confere (None quando íntegra).
    pub first_broken: Option<usize>,
    pub chain_head: String,
    /// Rodapé presente e coerente (apenas binário).
    pub footer_ok: bool,
    pub total_joules: f64,
    /// Contagem de eventos por kind.
    pub counts: BTreeMap<String, u64>,
    /// Atuações (ATUACAO) e quantas com sucesso.
    pub actuations: usize,
    pub atuacoes_ok: usize,
    /// Divergências de honestidade (alertas — falha de I/O, fallback…).
    pub alerts: usize,
}

/// Verifica um log do Caderno detectando o formato pelo magic.
pub fn verify(path: &Path) -> Result<VerificationReport, String> {
    let mut arquivo = std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut magic = [0u8; 4];
    let n_read = arquivo
        .read(&mut magic)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if n_read == 4 && &magic == MAGIC {
        verify_binary(path)
    } else {
        verify_jsonl(path)
    }
}

/// Verifica o formato binário `.vcad` v1.
pub fn verify_binary(path: &Path) -> Result<VerificationReport, String> {
    let mut data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if data.len() < 5 || &data[..4] != MAGIC {
        return Err(format!("{}: não é um Caderno binário v{VERSION}", path.display()));
    }
    if data[4] != VERSION {
        return Err(format!("{}: versão {} não suportada", path.display(), data[4]));
    }
    let mut pos = 5usize;
    let mut head = ChainLedger::INITIAL_HEAD.to_owned();
    let mut rel = VerificationReport {
        events: 0,
        chain_ok: true,
        first_broken: None,
        chain_head: String::new(),
        footer_ok: false,
        total_joules: 0.0,
        counts: BTreeMap::new(),
        actuations: 0,
        atuacoes_ok: 0,
        alerts: 0,
    };
    // Rodapé fixo (72 bytes): "VFIM" | eventos u32 LE | chain_head 64 ASCII.
    let mut footer_head = None;
    let mut events_footer: Option<u32> = None;
    if data.len() >= 5 + FOOTER_BYTES {
        let start = data.len() - FOOTER_BYTES;
        if &data[start..start + 4] == FOOTER_MAGIC {
            events_footer = Some(u32::from_le_bytes(
                data[start + 4..start + 8].try_into().map_err(|_| "rodapé truncado")?,
            ));
            if let Ok(h) = std::str::from_utf8(&data[start + 8..start + 8 + 64]) {
                footer_head = Some(h.to_owned());
            }
            rel.footer_ok = footer_head.is_some();
            // frames não invadem o rodapé
            data.truncate(start);
        }
    }
    while pos < data.len() {
        let Some(size_bytes) = data.get(pos..pos + 4) else {
            rel.chain_ok = false;
            break;
        };
        let size = u32::from_le_bytes(size_bytes.try_into().unwrap()) as usize;
        let Some(line_bytes) = data.get(pos + 4..pos + 4 + size) else {
            rel.chain_ok = false;
            rel.first_broken.get_or_insert(rel.events);
            break;
        };
        let Some(hash_bytes) = data.get(pos + 4 + size..pos + 4 + size + 32) else {
            rel.chain_ok = false;
            rel.first_broken.get_or_insert(rel.events);
            break;
        };
        let line = std::str::from_utf8(line_bytes)
            .map_err(|_| format!("{}: frame com UTF-8 inválido", path.display()))?;
        let expected = sha256_double_hex(head.as_bytes(), line.as_bytes());
        let read_hex: String =
            hash_bytes.iter().map(|b| format!("{b:02x}")).collect();
        if expected != read_hex {
            rel.chain_ok = false;
            rel.first_broken.get_or_insert(rel.events);
        }
        head = expected;
        acumular_stats(line, &mut rel);
        rel.events += 1;
        pos += 4 + size + 32;
    }
    rel.chain_head = head;
    if rel.chain_ok {
        if let Some(h) = &footer_head {
            if h != &rel.chain_head {
                rel.chain_ok = false;
                rel.footer_ok = false;
            }
        }
        if let Some(n) = events_footer {
            if n as usize != rel.events {
                rel.chain_ok = false;
                rel.footer_ok = false;
            }
        }
    }
    Ok(rel)
}

/// Verifica o export JSONL (um evento por linha, com `hash`).
pub fn verify_jsonl(path: &Path) -> Result<VerificationReport, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut head = ChainLedger::INITIAL_HEAD.to_owned();
    let mut rel = VerificationReport {
        events: 0,
        chain_ok: true,
        first_broken: None,
        chain_head: String::new(),
        footer_ok: true, // JSONL não tem rodapé
        total_joules: 0.0,
        counts: BTreeMap::new(),
        actuations: 0,
        atuacoes_ok: 0,
        alerts: 0,
    };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some(Json::Obj(fields)) = Json::parse(line) else {
            rel.chain_ok = false;
            rel.first_broken.get_or_insert(rel.events);
            break;
        };
        let kind = match fields.get("kind") {
            Some(Json::Str(s)) => s.clone(),
            _ => {
                rel.chain_ok = false;
                rel.first_broken.get_or_insert(rel.events);
                break;
            }
        };
        let msg = match fields.get("msg") {
            Some(Json::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let seq = match fields.get("seq") {
            Some(Json::Num(n)) => *n as usize,
            _ => rel.events,
        };
        let read_hash = match fields.get("hash") {
            Some(Json::Str(s)) => s.clone(),
            _ => {
                rel.chain_ok = false;
                rel.first_broken.get_or_insert(rel.events);
                break;
            }
        };
        // extra = todas as chaves exceto seq/kind/msg/hash
        let mut extra = BTreeMap::new();
        for (k, v) in &fields {
            if !matches!(k.as_str(), "seq" | "kind" | "msg" | "hash") {
                extra.insert(k.clone(), v.clone());
            }
        }
        let canonical_line =
            line_from(seq, &kind, &msg, &extra);
        let expected = sha256_double_hex(head.as_bytes(), canonical_line.as_bytes());
        if expected != read_hash {
            rel.chain_ok = false;
            rel.first_broken.get_or_insert(rel.events);
        }
        head = expected;
        // kinds v1 (PT) seguem aceitos: artefatos históricos permanecem verificáveis.
        if matches!(kind.as_str(), "LEAK" | "VAZAMENTO") {
            if let Some(Json::Num(j)) = fields.get("joules") {
                rel.total_joules += j;
            }
        }
        *rel.counts.entry(kind).or_insert(0) += 1;
        rel.events += 1;
    }
    rel.chain_head = head;
    Ok(rel)
}

/// Linha canônica a partir dos campos já separados (verificador JSONL).
fn line_from(seq: usize, kind: &str, msg: &str, extra: &BTreeMap<String, Json>) -> String {
    let mut line = format!("{seq}\u{1f}{kind}\u{1f}{msg}");
    if !extra.is_empty() {
        line.push('\u{1f}');
        line.push_str(&Json::Obj(extra.clone()).serialize());
    }
    line
}

/// Extrai estatísticas de uma linha canônica (`seq ␟ kind ␟ msg [␟ extra]`).
fn acumular_stats(line: &str, rel: &mut VerificationReport) {
    let mut parts = line.split('\u{1f}');
    let _seq = parts.next();
    let kind = parts.next().unwrap_or("");
    *rel.counts.entry(kind.to_owned()).or_insert(0) += 1;
    // v1 (PT) e v1.1 (EN) dos kinds contados: artefatos históricos continuam
    // produzindo estatísticas idênticas (NOTA DE VERSÃO em NOTEBOOK-FORMAT-v1.md).
    match kind {
        "LEAK" | "VAZAMENTO" => {
            if let Some(extra) = parts.next_back() {
                if let Some(Json::Obj(fields)) = Json::parse(extra) {
                    if let Some(Json::Num(j)) = fields.get("joules") {
                        rel.total_joules += j;
                    }
                }
            }
        }
        "ACTUATION" | "ATUACAO" => {
            rel.actuations += 1;
            if parts.next_back().is_some_and(|extra| {
                matches!(
                    Json::parse(extra),
                    Some(Json::Obj(c)) if matches!(c.get("sucesso"), Some(Json::Bool(true)))
                )
            }) {
                rel.atuacoes_ok += 1;
            }
        }
        "ALERT" | "ALERTA" => rel.alerts += 1,
        _ => {}
    }
}

/// Converte o binário `.vcad` para JSONL (auditoria externa textual).
/// Devolve o número de eventos convertidos.
pub fn jsonl_from_binary(binary: &Path, jsonl: &Path) -> Result<usize, String> {
    let data = std::fs::read(binary).map_err(|e| format!("{}: {e}", binary.display()))?;
    if data.len() < 5 || &data[..4] != MAGIC {
        return Err(format!("{}: não é um Caderno binário v{VERSION}", binary.display()));
    }
    let mut pos = 5usize;
    let mut file = std::fs::File::create(jsonl)
        .map_err(|e| format!("{}: {e}", jsonl.display()))?;
    let mut n = 0usize;
    while pos + 4 <= data.len() {
        // para no rodapé: frame truncado ⇒ era o rodapé
        let size = u32::from_le_bytes(
            data[pos..pos + 4].try_into().map_err(|_| "frame truncado")?,
        ) as usize;
        let Some(line_bytes) = data.get(pos + 4..pos + 4 + size) else {
            break;
        };
        let Some(hash_bytes) = data.get(pos + 4 + size..pos + 4 + size + 32) else {
            break; // rodapé (sem 32 bytes de hash)
        };
        let line = std::str::from_utf8(line_bytes)
            .map_err(|_| "frame com UTF-8 inválido".to_string())?;
        let jsonl_line = event_jsonl_from_line(line, hash_bytes)?;
        writeln!(file, "{jsonl_line}").map_err(|e| format!("{}: {e}", jsonl.display()))?;
        n += 1;
        pos += 4 + size + 32;
    }
    file.flush().map_err(|e| format!("{}: {e}", jsonl.display()))?;
    Ok(n)
}

/// Uma linha canônica → objeto JSONL (mesma forma de
/// `ChainLedger::export_jsonl`: seq/kind/msg + extra fundido + hash hex).
fn event_jsonl_from_line(line: &str, hash_raw: &[u8]) -> Result<String, String> {
    let mut parts = line.split('\u{1f}');
    let seq = parts.next().ok_or("linha sem seq")?;
    let kind = parts.next().ok_or("linha sem kind")?;
    let msg = parts.next().ok_or("linha sem msg")?;
    let extra = parts.next();
    let mut fields = BTreeMap::new();
    fields.insert("seq".to_owned(), Json::num(seq.parse::<f64>().map_err(|_| "seq inválido")?));
    fields.insert("kind".to_owned(), Json::str(kind));
    fields.insert("msg".to_owned(), Json::str(msg));
    if let Some(extra) = extra {
        if let Some(Json::Obj(mapa)) = Json::parse(extra) {
            for (k, v) in mapa {
                fields.insert(k, v);
            }
        }
    }
    let hash_hex: String = hash_raw.iter().map(|b| format!("{b:02x}")).collect();
    fields.insert("hash".to_owned(), Json::str(hash_hex));
    Ok(Json::Obj(fields).serialize())
}
