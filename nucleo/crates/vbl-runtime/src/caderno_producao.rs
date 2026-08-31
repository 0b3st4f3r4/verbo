//! Caderno de produção (Etapa 4 — PLAN §4.1; AGENTS §1.4).
//!
//! [`CadernoProducao`] é a implementação do trait [`Caderno`] para execuções
//! reais: gravação **assíncrona em buffer** (thread dedicada + canal; o loop
//! de tick só serializa e enfileira — PLAN §4.3 "overhead do Caderno pode
//! distorcer medições"), **formato binário compacto** `.vcad` (mesma filosofia
//! zero-dependência do schema FXP v1; Cap'n Proto/FlatBuffers citados no
//! AGENTS são exemplos, não exigência) e **agregados** para monitoramento.
//!
//! Honestidade: a thread de gravação NUNCA inventa eventos; a cadeia SHA-256
//! é incremental (só a cabeça fica em memória) e pode ser reavermelhada por
//! um agente externo com `vbl caderno-verify` (binário ou JSONL exportado).
//!
//! Formato `.vcad` v1 (spec: docs/CADERNO-FORMATO-v1.md):
//!
//! ```text
//! header : "VCAD" | versao u8
//! frame* : [u32 LE len][linha UTF-8 len bytes][hash SHA-256 — 32 bytes crus]
//! footer : "VFIM" | eventos u32 LE | chain_head 64 bytes ASCII (tamanho fixo 72)
//! ```
//!
//! `linha` é a MESMA linha canônica da cadeia (`seq ␟ kind ␟ msg [␟ extra_json]`
//! — ver [`crate::caderno::Evento::linha`]); o hash do frame é o elo da cadeia
//! em bytes crus. O verificador recomputa `hash_n = SHA-256(hash_{n-1} ||
//! linha_n)` — adulteração retroativa quebra a cadeia.

use crate::caderno::{carimbar_tempo, sha256_hex_duplo, Caderno, ChainCaderno};
use crate::json::{escrever_numero, escrever_string, Json};
use std::collections::BTreeMap;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

/// Magic do formato binário.
pub const MAGIC: &[u8; 4] = b"VCAD";
/// Magic do rodapé (tamanho fixo — leitura determinística pelo fim).
pub const RODAPE_MAGIC: &[u8; 4] = b"VFIM";
/// Versão do formato.
pub const VERSAO: u8 = 1;
/// Rodapé: magic (4) + eventos u32 (4) + chain_head 64 ASCII.
pub const RODAPE_BYTES: usize = 4 + 4 + 64;
/// Flush periódico: a cada N eventos (buffer + flush — PLAN §4.3).
const FLUSH_A_CADA: usize = 256;
/// Tamanho do buffer de escrita.
const BUFFER_BYTES: usize = 64 * 1024;

/// Mensagem tick-thread → thread de gravação. O carimbo de tempo viaja
/// solto: a thread de gravação injeta no `extra` antes de compor a linha.
///
/// Etapa 5: o caminho quente ([`Caderno::leak`] — um evento por forma por
/// tick) viaja como dados crus ([`Msg::Vazamento`]); a linha canônica é
/// composta na thread de gravação diretamente no buffer reutilizado, sem
/// `Json`/`BTreeMap` intermediários — bytes idênticos à composição geral
/// (garantido por teste de equivalência).
enum Msg {
    Evento { seq: usize, tick: u64, t: f64, kind: String, msg: String, extra: Json },
    Vazamento(Vazamento),
    Fim,
}

/// Dados crus do evento VAZAMENTO (caminho quente da Etapa 5) — a linha
/// canônica só existe na thread de gravação, direto no buffer reutilizado.
struct Vazamento {
    seq: usize,
    tick: u64,
    t: f64,
    forma: String,
    watts: f64,
    segundos: f64,
    joules: f64,
}

/// Agregados do Caderno de produção (expostos para monitoramento —
/// AGENTS §1.4: métricas agregadas, Joules totais e médias).
#[derive(Debug, Clone, Default)]
pub struct Resumo {
    pub eventos: usize,
    pub bytes: u64,
    pub chain_head: String,
    pub joules_totais: f64,
    pub joules_por_forma: BTreeMap<String, f64>,
    pub contagens: BTreeMap<String, u64>,
}

impl Resumo {
    /// Média de Joules por forma com vazamento registrado.
    pub fn media_joules_por_forma(&self) -> f64 {
        if self.joules_por_forma.is_empty() {
            0.0
        } else {
            self.joules_totais / self.joules_por_forma.len() as f64
        }
    }
}

/// Caderno de produção: eventos fluem por canal para uma thread de gravação
/// (BufWriter + flush periódico); em memória restam apenas a sequência, o
/// relógio, a potência corrente e os agregados — memória limitada mesmo com
/// 10.000 formas ativas (AGENTS §1.4).
#[derive(Debug)]
pub struct CadernoProducao {
    tx: Option<Sender<Msg>>,
    handle: Option<JoinHandle<std::io::Result<Resumo>>>,
    caminho: PathBuf,
    seq: usize,
    enfileirados: usize,
    tempo: (u64, f64),
    potencia: f64,
    joules_totais: f64,
    joules_por_forma: BTreeMap<String, f64>,
    contagens: BTreeMap<String, u64>,
}

impl CadernoProducao {
    /// Abre o arquivo binário (`.vcad`) e inicia a thread de gravação.
    pub fn abrir(caminho: impl Into<PathBuf>) -> std::io::Result<Self> {
        let caminho = caminho.into();
        let (tx, rec) = channel::<Msg>();
        let caminho_thread = caminho.clone();
        let handle = std::thread::Builder::new()
            .name("caderno".into())
            .spawn(move || thread_gravacao(rec, caminho_thread))?;
        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
            caminho,
            seq: 0,
            enfileirados: 0,
            tempo: (0, 0.0),
            potencia: 0.0,
            joules_totais: 0.0,
            joules_por_forma: BTreeMap::new(),
            contagens: BTreeMap::new(),
        })
    }

    /// Caminho do arquivo binário em gravação.
    pub fn caminho(&self) -> &Path {
        &self.caminho
    }

    /// Eventos enfileirados até agora (gravados ou na fila).
    pub fn enfileirados(&self) -> usize {
        self.enfileirados
    }

    /// Encerra: sinaliza o fim, espera a thread drenar a fila e devolve os
    /// agregados. Idempotente via consumo de `self`.
    pub fn fechar(mut self) -> Result<Resumo, String> {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Fim); // erro ⇒ thread já morreu (relatado abaixo)
        }
        let resumo_escrita = self
            .handle
            .take()
            .expect("handle presente até fechar")
            .join()
            .map_err(|_| "thread de gravação do Caderno panicou".to_string())?
            .map_err(|e| format!("gravação do Caderno ({}): {e}", self.caminho.display()))?;
        debug_assert_eq!(resumo_escrita.eventos, self.enfileirados);
        Ok(Resumo {
            eventos: resumo_escrita.eventos,
            bytes: resumo_escrita.bytes,
            chain_head: resumo_escrita.chain_head,
            joules_totais: self.joules_totais,
            joules_por_forma: std::mem::take(&mut self.joules_por_forma),
            contagens: std::mem::take(&mut self.contagens),
        })
    }
}

impl Drop for CadernoProducao {
    fn drop(&mut self) {
        // fechar() explícito é o caminho normal; o Drop garante que a thread
        // termina e o arquivo fica válido mesmo em caminhos de erro (sem
        // vazar thread nem arquivo truncado — AGENTS §1.3).
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Fim);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Caderno for CadernoProducao {
    fn definir_tempo(&mut self, tick: u64, t: f64) {
        self.tempo = (tick, t);
    }

    fn definir_potencia(&mut self, watts: f64) {
        self.potencia = watts;
    }

    fn potencia_corrente(&self) -> Option<f64> {
        Some(self.potencia)
    }

    fn record(&mut self, kind: &str, msg: &str, extra: Json) {
        // caminho geral (eventos raros: transições, alertas…) — acumula sem
        // alocar a chave quando o kind já está na tabela (Etapa 5)
        match self.contagens.get_mut(kind) {
            Some(c) => *c += 1,
            None => {
                self.contagens.insert(kind.to_owned(), 1);
            }
        }
        let seq = self.seq;
        self.seq += 1;
        self.enfileirados += 1;
        if let Some(tx) = &self.tx {
            // unbounded: a thread grava em buffer de 64 KiB; sobrecarga de
            // disco aparece como memória na fila, nunca como perda de evento
            // (cobertura de eventos 100% — AGENTS §1.4).
            let _ = tx.send(Msg::Evento {
                seq,
                tick: self.tempo.0,
                t: self.tempo.1,
                kind: kind.to_owned(),
                msg: msg.to_owned(),
                extra,
            });
        }
    }

    /// Caminho quente (Etapa 5 — PLAN §5.2): acumula os Joules e envia os
    /// dados crus; a linha canônica é composta na thread de gravação sem
    /// `format!`+`Json::obj` intermediários (o custo dominante medido na
    /// Etapa 4 — docs/ETAPA-4-RELATORIO.md §3). Agregados e contagens têm o
    /// MESMO contrato do caminho geral.
    fn leak(&mut self, forma: &str, watts: f64, segundos: f64) {
        let joules = watts * segundos;
        self.joules_totais += joules;
        match self.joules_por_forma.get_mut(forma) {
            Some(j) => *j += joules,
            None => {
                self.joules_por_forma.insert(forma.to_owned(), joules);
            }
        }
        match self.contagens.get_mut("VAZAMENTO") {
            Some(c) => *c += 1,
            None => {
                self.contagens.insert("VAZAMENTO".to_owned(), 1);
            }
        }
        let seq = self.seq;
        self.seq += 1;
        self.enfileirados += 1;
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Vazamento(Vazamento {
                seq,
                tick: self.tempo.0,
                t: self.tempo.1,
                forma: forma.to_owned(),
                watts,
                segundos,
                joules,
            }));
        }
    }
}

/// Corpo da thread de gravação: header, frames, footer.
///
/// Etapa 5: linha e cabeça da cadeia vivem em buffers REUTILIZADOS (um único
/// `String` de linha para todo o log) e o elo é calculado sobre duas fatias
/// com digest cru — sem concatenar `head+linha` nem fazer a viagem hex → cru
/// de cada frame (docs/ETAPA-4-RELATORIO.md §3: encoding direto).
fn thread_gravacao(rec: Receiver<Msg>, caminho: PathBuf) -> std::io::Result<Resumo> {
    let arquivo = std::fs::File::create(&caminho)?;
    let mut w = BufWriter::with_capacity(BUFFER_BYTES, arquivo);
    w.write_all(MAGIC)?;
    w.write_all(&[VERSAO])?;

    let mut head = ChainCaderno::HEAD_INICIAL.to_owned();
    let mut linha = String::with_capacity(192);
    let mut eventos = 0usize;
    let mut bytes = 0u64;
    while let Ok(msg) = rec.recv() {
        match msg {
            Msg::Fim => break,
            Msg::Evento { seq, tick, t, kind, msg, mut extra } => {
                carimbar_tempo(&mut extra, tick, t);
                let evento =
                    crate::caderno::Evento { seq, kind, msg, extra, hash: String::new() };
                linha.clear();
                evento.escrever_linha(&mut linha);
            }
            Msg::Vazamento(v) => {
                linha.clear();
                escrever_linha_vazamento(&mut linha, &v);
            }
        }
        let digest = crate::caderno::sha256_bytes_duplo(head.as_bytes(), linha.as_bytes());
        w.write_all(&(linha.len() as u32).to_le_bytes())?;
        w.write_all(linha.as_bytes())?;
        w.write_all(&digest)?;
        head.clear();
        crate::caderno::escrever_hex(&digest, &mut head);
        eventos += 1;
        bytes += 4 + linha.len() as u64 + 32;
        if eventos.is_multiple_of(FLUSH_A_CADA) {
            w.flush()?;
        }
    }
    w.write_all(RODAPE_MAGIC)?;
    w.write_all(&(eventos as u32).to_le_bytes())?;
    w.write_all(head.as_bytes())?; // 64 bytes hex
    bytes += RODAPE_BYTES as u64;
    w.flush()?;
    Ok(Resumo { eventos, bytes, chain_head: head, ..Default::default() })
}

/// Composição direta da linha canônica do evento VAZAMENTO — byte a byte
/// idêntica a `evento_vazamento` + `carimbar_tempo` + `Evento::linha`:
/// `seq ␟ VAZAMENTO ␟ msg ␟ {"forma","joules","segundos","t","tick","watts"}`
/// (chaves em ordem de classificação; números no formato canônico do `Json`).
/// Equivalência garantida por teste (`tests/caderno_producao.rs`).
fn escrever_linha_vazamento(linha: &mut String, v: &Vazamento) {
    use std::fmt::Write as _;
    let _ = write!(
        linha,
        "{seq}\u{1f}VAZAMENTO\u{1f}Forma '{forma}' dissipou {joules:.2} Joules ({watts:.2} W por {segundos:.2}s)\u{1f}",
        seq = v.seq,
        forma = v.forma,
        joules = v.joules,
        watts = v.watts,
        segundos = v.segundos,
    );
    linha.push_str("{\"forma\":");
    escrever_string(&v.forma, linha);
    linha.push_str(",\"joules\":");
    escrever_numero(v.joules, linha);
    linha.push_str(",\"segundos\":");
    escrever_numero(v.segundos, linha);
    linha.push_str(",\"t\":");
    escrever_numero(v.t, linha);
    linha.push_str(",\"tick\":");
    escrever_numero(v.tick as f64, linha);
    linha.push_str(",\"watts\":");
    escrever_numero(v.watts, linha);
    linha.push('}');
}

// ======================================================================
// Verificação externa (agente externo — AGENTS §1.4: checksums SHA-256)
// ======================================================================

/// Relatório da verificação de um log do Caderno (binário ou JSONL).
#[derive(Debug, Clone)]
pub struct RelatorioVerificacao {
    pub eventos: usize,
    pub cadeia_ok: bool,
    /// Primeiro evento cujo hash não confere (None quando íntegra).
    pub primeiro_quebrado: Option<usize>,
    pub chain_head: String,
    /// Rodapé presente e coerente (apenas binário).
    pub rodape_ok: bool,
    pub joules_totais: f64,
    /// Contagem de eventos por kind.
    pub contagens: BTreeMap<String, u64>,
    /// Atuações (ATUACAO) e quantas com sucesso.
    pub atuacoes: usize,
    pub atuacoes_ok: usize,
    /// Divergências de honestidade (alertas — falha de I/O, fallback…).
    pub alertas: usize,
}

/// Verifica um log do Caderno detectando o formato pelo magic.
pub fn verificar(caminho: &Path) -> Result<RelatorioVerificacao, String> {
    let mut arquivo = std::fs::File::open(caminho)
        .map_err(|e| format!("{}: {e}", caminho.display()))?;
    let mut magic = [0u8; 4];
    let lido = arquivo
        .read(&mut magic)
        .map_err(|e| format!("{}: {e}", caminho.display()))?;
    if lido == 4 && &magic == MAGIC {
        verificar_binario(caminho)
    } else {
        verificar_jsonl(caminho)
    }
}

/// Verifica o formato binário `.vcad` v1.
pub fn verificar_binario(caminho: &Path) -> Result<RelatorioVerificacao, String> {
    let mut dados = std::fs::read(caminho).map_err(|e| format!("{}: {e}", caminho.display()))?;
    if dados.len() < 5 || &dados[..4] != MAGIC {
        return Err(format!("{}: não é um Caderno binário v{VERSAO}", caminho.display()));
    }
    if dados[4] != VERSAO {
        return Err(format!("{}: versão {} não suportada", caminho.display(), dados[4]));
    }
    let mut pos = 5usize;
    let mut head = ChainCaderno::HEAD_INICIAL.to_owned();
    let mut rel = RelatorioVerificacao {
        eventos: 0,
        cadeia_ok: true,
        primeiro_quebrado: None,
        chain_head: String::new(),
        rodape_ok: false,
        joules_totais: 0.0,
        contagens: BTreeMap::new(),
        atuacoes: 0,
        atuacoes_ok: 0,
        alertas: 0,
    };
    // Rodapé fixo (72 bytes): "VFIM" | eventos u32 LE | chain_head 64 ASCII.
    let mut rodape_head = None;
    let mut rodape_eventos: Option<u32> = None;
    if dados.len() >= 5 + RODAPE_BYTES {
        let inicio = dados.len() - RODAPE_BYTES;
        if &dados[inicio..inicio + 4] == RODAPE_MAGIC {
            rodape_eventos = Some(u32::from_le_bytes(
                dados[inicio + 4..inicio + 8].try_into().map_err(|_| "rodapé truncado")?,
            ));
            if let Ok(h) = std::str::from_utf8(&dados[inicio + 8..inicio + 8 + 64]) {
                rodape_head = Some(h.to_owned());
            }
            rel.rodape_ok = rodape_head.is_some();
            // frames não invadem o rodapé
            dados.truncate(inicio);
        }
    }
    while pos < dados.len() {
        let Some(tam_bytes) = dados.get(pos..pos + 4) else {
            rel.cadeia_ok = false;
            break;
        };
        let tam = u32::from_le_bytes(tam_bytes.try_into().unwrap()) as usize;
        let Some(linha_bytes) = dados.get(pos + 4..pos + 4 + tam) else {
            rel.cadeia_ok = false;
            rel.primeiro_quebrado.get_or_insert(rel.eventos);
            break;
        };
        let Some(hash_bytes) = dados.get(pos + 4 + tam..pos + 4 + tam + 32) else {
            rel.cadeia_ok = false;
            rel.primeiro_quebrado.get_or_insert(rel.eventos);
            break;
        };
        let linha = std::str::from_utf8(linha_bytes)
            .map_err(|_| format!("{}: frame com UTF-8 inválido", caminho.display()))?;
        let esperado = sha256_hex_duplo(head.as_bytes(), linha.as_bytes());
        let lido_hex: String =
            hash_bytes.iter().map(|b| format!("{b:02x}")).collect();
        if esperado != lido_hex {
            rel.cadeia_ok = false;
            rel.primeiro_quebrado.get_or_insert(rel.eventos);
        }
        head = esperado;
        acumular_stats(linha, &mut rel);
        rel.eventos += 1;
        pos += 4 + tam + 32;
    }
    rel.chain_head = head;
    if rel.cadeia_ok {
        if let Some(h) = &rodape_head {
            if h != &rel.chain_head {
                rel.cadeia_ok = false;
                rel.rodape_ok = false;
            }
        }
        if let Some(n) = rodape_eventos {
            if n as usize != rel.eventos {
                rel.cadeia_ok = false;
                rel.rodape_ok = false;
            }
        }
    }
    Ok(rel)
}

/// Verifica o export JSONL (um evento por linha, com `hash`).
pub fn verificar_jsonl(caminho: &Path) -> Result<RelatorioVerificacao, String> {
    let texto = std::fs::read_to_string(caminho)
        .map_err(|e| format!("{}: {e}", caminho.display()))?;
    let mut head = ChainCaderno::HEAD_INICIAL.to_owned();
    let mut rel = RelatorioVerificacao {
        eventos: 0,
        cadeia_ok: true,
        primeiro_quebrado: None,
        chain_head: String::new(),
        rodape_ok: true, // JSONL não tem rodapé
        joules_totais: 0.0,
        contagens: BTreeMap::new(),
        atuacoes: 0,
        atuacoes_ok: 0,
        alertas: 0,
    };
    for linha in texto.lines().filter(|l| !l.trim().is_empty()) {
        let Some(Json::Obj(campos)) = Json::analisar(linha) else {
            rel.cadeia_ok = false;
            rel.primeiro_quebrado.get_or_insert(rel.eventos);
            break;
        };
        let kind = match campos.get("kind") {
            Some(Json::Str(s)) => s.clone(),
            _ => {
                rel.cadeia_ok = false;
                rel.primeiro_quebrado.get_or_insert(rel.eventos);
                break;
            }
        };
        let msg = match campos.get("msg") {
            Some(Json::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let seq = match campos.get("seq") {
            Some(Json::Num(n)) => *n as usize,
            _ => rel.eventos,
        };
        let hash_lido = match campos.get("hash") {
            Some(Json::Str(s)) => s.clone(),
            _ => {
                rel.cadeia_ok = false;
                rel.primeiro_quebrado.get_or_insert(rel.eventos);
                break;
            }
        };
        // extra = todas as chaves exceto seq/kind/msg/hash
        let mut extra = BTreeMap::new();
        for (k, v) in &campos {
            if !matches!(k.as_str(), "seq" | "kind" | "msg" | "hash") {
                extra.insert(k.clone(), v.clone());
            }
        }
        let linha_canonica =
            linha_de(seq, &kind, &msg, &extra);
        let esperado = sha256_hex_duplo(head.as_bytes(), linha_canonica.as_bytes());
        if esperado != hash_lido {
            rel.cadeia_ok = false;
            rel.primeiro_quebrado.get_or_insert(rel.eventos);
        }
        head = esperado;
        if kind == "VAZAMENTO" {
            if let Some(Json::Num(j)) = campos.get("joules") {
                rel.joules_totais += j;
            }
        }
        *rel.contagens.entry(kind).or_insert(0) += 1;
        rel.eventos += 1;
    }
    rel.chain_head = head;
    Ok(rel)
}

/// Linha canônica a partir dos campos já separados (verificador JSONL).
fn linha_de(seq: usize, kind: &str, msg: &str, extra: &BTreeMap<String, Json>) -> String {
    let mut linha = format!("{seq}\u{1f}{kind}\u{1f}{msg}");
    if !extra.is_empty() {
        linha.push('\u{1f}');
        linha.push_str(&Json::Obj(extra.clone()).serializar());
    }
    linha
}

/// Extrai estatísticas de uma linha canônica (`seq ␟ kind ␟ msg [␟ extra]`).
fn acumular_stats(linha: &str, rel: &mut RelatorioVerificacao) {
    let mut partes = linha.split('\u{1f}');
    let _seq = partes.next();
    let kind = partes.next().unwrap_or("");
    *rel.contagens.entry(kind.to_owned()).or_insert(0) += 1;
    match kind {
        "VAZAMENTO" => {
            if let Some(extra) = partes.next_back() {
                if let Some(Json::Obj(campos)) = Json::analisar(extra) {
                    if let Some(Json::Num(j)) = campos.get("joules") {
                        rel.joules_totais += j;
                    }
                }
            }
        }
        "ATUACAO" => {
            rel.atuacoes += 1;
            if partes.next_back().is_some_and(|extra| {
                matches!(
                    Json::analisar(extra),
                    Some(Json::Obj(c)) if matches!(c.get("sucesso"), Some(Json::Bool(true)))
                )
            }) {
                rel.atuacoes_ok += 1;
            }
        }
        "ALERTA" => rel.alertas += 1,
        _ => {}
    }
}

/// Converte o binário `.vcad` para JSONL (auditoria externa textual).
/// Devolve o número de eventos convertidos.
pub fn jsonl_de_binario(binario: &Path, jsonl: &Path) -> Result<usize, String> {
    let dados = std::fs::read(binario).map_err(|e| format!("{}: {e}", binario.display()))?;
    if dados.len() < 5 || &dados[..4] != MAGIC {
        return Err(format!("{}: não é um Caderno binário v{VERSAO}", binario.display()));
    }
    let mut pos = 5usize;
    let mut ficheiro = std::fs::File::create(jsonl)
        .map_err(|e| format!("{}: {e}", jsonl.display()))?;
    let mut n = 0usize;
    while pos + 4 <= dados.len() {
        // para no rodapé: frame truncado ⇒ era o rodapé
        let tam = u32::from_le_bytes(
            dados[pos..pos + 4].try_into().map_err(|_| "frame truncado")?,
        ) as usize;
        let Some(linha_bytes) = dados.get(pos + 4..pos + 4 + tam) else {
            break;
        };
        let Some(hash_bytes) = dados.get(pos + 4 + tam..pos + 4 + tam + 32) else {
            break; // rodapé (sem 32 bytes de hash)
        };
        let linha = std::str::from_utf8(linha_bytes)
            .map_err(|_| "frame com UTF-8 inválido".to_string())?;
        let jsonl_linha = evento_jsonl_de_linha(linha, hash_bytes)?;
        writeln!(ficheiro, "{jsonl_linha}").map_err(|e| format!("{}: {e}", jsonl.display()))?;
        n += 1;
        pos += 4 + tam + 32;
    }
    ficheiro.flush().map_err(|e| format!("{}: {e}", jsonl.display()))?;
    Ok(n)
}

/// Uma linha canônica → objeto JSONL (mesma forma de
/// `ChainCaderno::export_jsonl`: seq/kind/msg + extra fundido + hash hex).
fn evento_jsonl_de_linha(linha: &str, hash_cru: &[u8]) -> Result<String, String> {
    let mut partes = linha.split('\u{1f}');
    let seq = partes.next().ok_or("linha sem seq")?;
    let kind = partes.next().ok_or("linha sem kind")?;
    let msg = partes.next().ok_or("linha sem msg")?;
    let extra = partes.next();
    let mut campos = BTreeMap::new();
    campos.insert("seq".to_owned(), Json::num(seq.parse::<f64>().map_err(|_| "seq inválido")?));
    campos.insert("kind".to_owned(), Json::str(kind));
    campos.insert("msg".to_owned(), Json::str(msg));
    if let Some(extra) = extra {
        if let Some(Json::Obj(mapa)) = Json::analisar(extra) {
            for (k, v) in mapa {
                campos.insert(k, v);
            }
        }
    }
    let hash_hex: String = hash_cru.iter().map(|b| format!("{b:02x}")).collect();
    campos.insert("hash".to_owned(), Json::str(hash_hex));
    Ok(Json::Obj(campos).serializar())
}
