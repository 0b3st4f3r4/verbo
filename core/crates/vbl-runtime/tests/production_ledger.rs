//! Caderno de produção (Etapa 4 — PLAN §4.1; AGENTS §1.4): gravação
//! assíncrona, formato binário `.vcad`, timestamps do relógio virtual,
//! atuação com custo estimado, verificação externa e estresse 10k formas.

use std::io::Write;
use std::path::PathBuf;
use vbl_runtime::fxp::Value;
use vbl_runtime::json::Json;
use vbl_runtime::ledger::{Actuation, ChainLedger, Ledger};
use vbl_runtime::production_ledger::{
    jsonl_from_binary, verify, verify_binary, verify_jsonl, ProductionLedger,
};
use vbl_runtime::{load, Engine, FxpSimulator};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vbl-caderno-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ======================================================================
// Gravação assíncrona + integridade (cadeia SHA-256)
// ======================================================================

#[test]
fn async_write_produces_intact_chain() {
    let dir = temp_dir("integra");
    let path = dir.join("caderno.vcad");
    let mut ledger = ProductionLedger::open(&path).unwrap();
    ledger.set_time(1, 1.0);
    ledger.record(
        "INFO",
        "primeiro evento",
        Json::obj([("forma", Json::str("A"))]),
    );
    ledger.set_time(2, 2.0);
    ledger.record(
        "INFO",
        "segundo evento",
        Json::obj([("forma", Json::str("B"))]),
    );
    let summary = ledger.close().unwrap();
    assert_eq!(summary.events, 2);
    assert!(summary.bytes > 5 + 4);
    assert_ne!(summary.chain_head, ChainLedger::INITIAL_HEAD);

    let rel = verify_binary(&path).unwrap();
    assert_eq!(rel.events, 2);
    assert!(rel.chain_ok, "cadeia deve estar íntegra");
    assert!(rel.first_broken.is_none());
    assert!(rel.footer_ok, "rodapé presente e coerente");
    assert_eq!(rel.chain_head, summary.chain_head);
    assert_eq!(rel.counts.get("INFO"), Some(&2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn binary_tampering_breaks_chain() {
    let dir = temp_dir("adulterado");
    let path = dir.join("caderno.vcad");
    let mut ledger = ProductionLedger::open(&path).unwrap();
    ledger.record("INFO", "evento original", Json::obj([]));
    ledger.record("INFO", "outro evento", Json::obj([]));
    ledger.close().unwrap();

    let mut data = std::fs::read(&path).unwrap();
    // adulteração retroativa no primeiro frame (payload: header 5 bytes + len 4)
    let alvo = 9 + 4; // dentro do campo "kind"/"msg" da linha 0
    data[alvo] = b'X';
    std::fs::write(&path, &data).unwrap();

    let rel = verify_binary(&path).unwrap();
    assert!(!rel.chain_ok, "adulteração deve quebrar a cadeia");
    assert_eq!(rel.first_broken, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jsonl_from_binary_reproduces_chain() {
    let dir = temp_dir("jsonl");
    let binary = dir.join("caderno.vcad");
    let jsonl = dir.join("caderno.jsonl");
    let mut ledger = ProductionLedger::open(&binary).unwrap();
    ledger.set_time(3, 3.0);
    ledger.record(
        "INFO",
        "com timestamps",
        Json::obj([("forma", Json::str("A"))]),
    );
    ledger.close().unwrap();

    let n = jsonl_from_binary(&binary, &jsonl).unwrap();
    assert_eq!(n, 1);
    let rel_bin = verify_binary(&binary).unwrap();
    let rel_jsonl = verify_jsonl(&jsonl).unwrap();
    assert!(rel_jsonl.chain_ok);
    assert_eq!(
        rel_bin.chain_head, rel_jsonl.chain_head,
        "mesma cadeia nos dois formatos"
    );

    // o JSONL exporta os timestamps no nível superior (AGENTS §1.4)
    let text = std::fs::read_to_string(&jsonl).unwrap();
    assert!(
        text.contains("\"tick\":3"),
        "tick do relógio virtual no JSONL: {text}"
    );
    assert!(
        text.contains("\"t\":3"),
        "t (segundos virtuais) no JSONL: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tampered_jsonl_breaks_chain() {
    let dir = temp_dir("jsonl-adulterado");
    let jsonl = dir.join("caderno.jsonl");
    let mut ledger = ChainLedger::new();
    ledger.record("INFO", "linha original", Json::obj([]));
    ledger.export_jsonl(&jsonl).unwrap();

    let mut text = std::fs::read_to_string(&jsonl).unwrap();
    text = text.replace("linha original", "linha FORJADA");
    std::fs::write(&jsonl, &text).unwrap();

    let rel = verify_jsonl(&jsonl).unwrap();
    assert!(!rel.chain_ok);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn automatic_format_detection() {
    let dir = temp_dir("deteccao");
    let binary = dir.join("a.vcad");
    let jsonl = dir.join("a.jsonl");
    let mut ledger = ProductionLedger::open(&binary).unwrap();
    ledger.record("INFO", "evento", Json::obj([]));
    ledger.close().unwrap();
    jsonl_from_binary(&binary, &jsonl).unwrap();
    // `verify` escolhe pelo magic: binário e JSONL íntegros, mesma cadeia
    let a = verify(&binary).unwrap();
    let b = verify(&jsonl).unwrap();
    assert!(a.chain_ok && b.chain_ok);
    assert_eq!(a.chain_head, b.chain_head);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drop_without_close_writes_valid_footer() {
    let dir = temp_dir("drop");
    let path = dir.join("caderno.vcad");
    {
        let mut ledger = ProductionLedger::open(&path).unwrap();
        ledger.record("INFO", "antes do drop", Json::obj([]));
    } // Drop: encerra a thread e fecha o arquivo
    let rel = verify_binary(&path).unwrap();
    assert!(rel.chain_ok);
    assert_eq!(rel.events, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Agregados (Joules por forma — AGENTS §1.4)
// ======================================================================

#[test]
fn joules_per_form_aggregates() {
    let dir = temp_dir("joules");
    let path = dir.join("caderno.vcad");
    let mut ledger = ProductionLedger::open(&path).unwrap();
    ledger.leak("A", 30.0, 1.0);
    ledger.leak("A", 10.0, 1.0);
    ledger.leak("B", 60.0, 2.0);
    let summary = ledger.close().unwrap();
    assert!((summary.total_joules - 160.0).abs() < 1e-9);
    assert!((summary.joules_per_form["A"] - 40.0).abs() < 1e-9);
    assert!((summary.joules_per_form["B"] - 120.0).abs() < 1e-9);
    assert!((summary.avg_joules_per_form() - 80.0).abs() < 1e-9);
    // os eventos VAZAMENTO também estão no arquivo, com os mesmos Joules
    let rel = verify_binary(&path).unwrap();
    assert!((rel.total_joules - 160.0).abs() < 1e-9);
    assert_eq!(rel.counts.get("LEAK"), Some(&3));
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Atuação com valor aplicado, latência e custo energético (PLAN §4.1)
// ======================================================================

#[test]
fn actuation_records_applied_latency_and_cost() {
    let dir = temp_dir("atuacao");
    let path = dir.join("caderno.vcad");
    let mut ledger = ProductionLedger::open(&path).unwrap();
    ledger.set_power(100.0); // potência do tick (W)
    ledger.actuator_action_detailed(Actuation {
        actor: "CpuPowerCap".into(),
        requested: Value::Num(50.0),
        applied: Some(Value::Num(50.0)),
        latency_us: Some(250),
        joule_cost: None, // estimado: 100 W × 250 µs = 0.025 J
        success: true,
    });
    ledger.actuator_action_detailed(Actuation {
        actor: "Fan".into(),
        requested: Value::Num(255.0),
        applied: None,
        latency_us: None,
        joule_cost: None,
        success: false,
    });
    ledger.close().unwrap();

    let rel = verify_binary(&path).unwrap();
    assert_eq!(rel.actuations, 2);
    assert_eq!(rel.atuacoes_ok, 1);
    // confere os campos no JSONL exportado
    let jsonl = dir.join("caderno.jsonl");
    jsonl_from_binary(&path, &jsonl).unwrap();
    let text = std::fs::read_to_string(&jsonl).unwrap();
    let actuation_line = text.lines().next().unwrap();
    assert!(
        actuation_line.contains("\"aplicado\":50"),
        "{actuation_line}"
    );
    assert!(
        actuation_line.contains("\"latencia_us\":250"),
        "{actuation_line}"
    );
    assert!(
        actuation_line.contains("\"custo_estimado_joules\":0.025"),
        "custo = potência × latência: {actuation_line}"
    );
    // falha não tem valor aplicado nem custo (nada foi aplicado)
    let failure_line = text.lines().nth(1).unwrap();
    assert!(!failure_line.contains("\"aplicado\""), "{failure_line}");
    assert!(
        !failure_line.contains("\"custo_estimado_joules\""),
        "{failure_line}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Engine + Caderno de produção (integração)
// ======================================================================

#[test]
fn engine_with_production_ledger_stamps_virtual_clock() {
    let dir = temp_dir("engine-relogio");
    let path = dir.join("caderno.vcad");
    let sim = FxpSimulator::new();
    let mut engine = Engine::with_ledger(
        sim,
        1.0,
        dir.join("persistence"),
        ProductionLedger::open(&path).unwrap(),
    );
    let (program, diags) = vbl_lang::parse("event X { value: \"v\", horizon: 3s }");
    assert!(diags.items.is_empty());
    let _interp = load(&mut engine, &program);
    engine.tick();
    drop(engine); // fecha a thread de gravação

    let jsonl = dir.join("caderno.jsonl");
    jsonl_from_binary(&path, &jsonl).unwrap();
    let text = std::fs::read_to_string(&jsonl).unwrap();
    // eventos do tick 1 carregam tick=1 e t=1 (relógio virtual — AGENTS §1.4)
    assert!(text.contains("\"tick\":1"), "{text}");
    assert!(text.contains("\"t\":1"), "{text}");
    let rel = verify_binary(&path).unwrap();
    assert!(rel.chain_ok);
    assert!(rel.events > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Estresse: 10.000 formas ativas (AGENTS §1.4/§1.5; PLAN §4.2)
// ======================================================================

#[test]
fn stress_10k_forms_all_events_recorded() {
    const FORMS: usize = 10_000;
    const TICKS: usize = 5;
    let dir = temp_dir("estresse");
    let path = dir.join("caderno.vcad");
    let sim = FxpSimulator::new();
    let mut engine = Engine::with_ledger(
        sim,
        1.0,
        dir.join("persistence"),
        ProductionLedger::open(&path).unwrap(),
    );
    let mut source = String::new();
    for i in 0..FORMS {
        source.push_str(&format!(
            "event F{i} {{ value: \"v{i}\", horizon: 1000000s }}\n"
        ));
    }
    let (program, diags) = vbl_lang::parse(&source);
    assert!(!diags.has_errors());
    let _interp = load(&mut engine, &program);
    for _ in 0..TICKS {
        engine.tick();
    }
    let enqueued = engine.ledger.enqueued();
    // ≥ 1 VAZAMENTO por forma por tick (10k × 5 = 50k) + registros de carga
    assert!(enqueued >= FORMS * TICKS);
    drop(engine);

    let rel = verify_binary(&path).unwrap();
    assert_eq!(rel.events, enqueued, "cobertura de eventos: 100%");
    assert!(rel.chain_ok, "cadeia íntegra sob carga máxima");
    assert!(rel.total_joules > 0.0);
    // robustez: nenhum evento perdido na fila (AGENTS §1.4 — 99,99%+)
    assert_eq!(rel.counts.get("LEAK"), Some(&((FORMS * TICKS) as u64)));
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Parser JSON mínimo (base do verificador)
// ======================================================================

#[test]
fn deterministic_json_parser_roundtrip() {
    let original = Json::obj([
        ("seq", Json::num(7.0)),
        ("kind", Json::str("ACTUATION")),
        ("msg", Json::str("Ator 'Fan' <- 200 (sucesso)")),
        ("valor", Json::num(200.0)),
        ("decimal", Json::num(0.5)),
        ("negativo", Json::num(-3.25)),
        (
            "lista",
            Json::Arr(vec![Json::str("a"), Json::boolean(true), Json::Null]),
        ),
        (
            "objeto",
            Json::obj([("chave", Json::str("valor \"escapado\""))]),
        ),
    ]);
    let text = original.serialize();
    let n_read = Json::parse(&text).expect("parser deve ler o que o serializador escreve");
    assert_eq!(n_read.serialize(), text, "roundtrip estável");
    assert!(Json::parse(&format!("{text} lixo")).is_none());
    assert!(Json::parse("{\"a\":}").is_none());
}

/// Escrita direta de bytes (uso do testes de tolerância a truncagem).
#[allow(dead_code)]
fn write_bytes(path: &std::path::Path, data: &[u8]) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(data).unwrap();
}

#[test]
fn truncated_file_rejected_with_clear_error() {
    let dir = temp_dir("truncado");
    let path = dir.join("caderno.vcad");
    let mut ledger = ProductionLedger::open(&path).unwrap();
    ledger.record("INFO", "evento", Json::obj([]));
    ledger.close().unwrap();
    // trunca no meio do primeiro frame (sem hash completo)
    let data = std::fs::read(&path).unwrap();
    std::fs::write(&path, &data[..data.len() - 40]).unwrap();
    let rel = verify_binary(&path).unwrap();
    assert!(!rel.chain_ok, "truncagem deve quebrar a cadeia");
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Etapa 5 — equivalência do caminho direto (encoding sem Json)
// ======================================================================

/// Extrai as linhas canônicas dos frames de um `.vcad` (u32 LE len + linha).
fn vcad_lines(path: &std::path::Path) -> Vec<String> {
    let data = std::fs::read(path).unwrap();
    let mut pos = 5usize; // header: "VCAD" + versão
    let mut lines = Vec::new();
    while pos + 4 <= data.len() {
        let size = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let Some(line) = data.get(pos + 4..pos + 4 + size) else {
            break;
        };
        let Some(_) = data.get(pos + 4 + size..pos + 4 + size + 32) else {
            break;
        };
        lines.push(String::from_utf8(line.to_vec()).unwrap());
        pos += 4 + size + 32;
    }
    lines
}

/// Etapa 5 (PLAN §5.2): o caminho quente `leak` → `Msg::Leak` →
/// composição direta no buffer produz linhas BYTE A BYTE idênticas às do
/// caminho geral (`leak_event` + `stamp_time` + `LedgerEvent::line`).
/// Referência independente: a mesma sequência de `leak` num `ChainLedger`
/// (implementação de referência, caminho geral) — os casos cobrem escape de
/// aspas/barra/controle, unicode, 0.0 W válido (FORMAL §4.7), negativos e
/// valores integrais (formato canônico de números). Nomes são identificadores
/// da gramática (sem `\u{1f}` — o separador canônico da linha).
#[test]
fn leak_direct_path_identical_to_general_composition() {
    let casos: [(&str, f64, f64, u64, f64); 7] = [
        ("F1", 0.15, 1.0, 1, 1.0),
        ("forma com espaço", 45.0, 1.0, 2, 2.0),
        ("aspas\"e\\barra", 85.5, 0.5, 3, 3.5),
        ("unicode_oxidado\u{1}", 0.0, 1.0, 4, 4.0),
        ("negativo", -3.25, 2.0, 5, 5.0),
        ("inteiro", 45.0, 2.0, 6, 6.0),
        ("temperatura_média", 12.75, 0.25, 7, 7.25),
    ];
    let dir = temp_dir("equivalencia");
    let path = dir.join("caderno.vcad");

    // referência: caminho GERAL (implementação de referência em memória)
    let mut reference = ChainLedger::new();
    // direto: caminho quente do Caderno de produção
    let mut production = ProductionLedger::open(&path).unwrap();
    for (form, watts, seconds, tick, t) in casos {
        reference.set_time(tick, t);
        production.set_time(tick, t);
        reference.leak(form, watts, seconds);
        production.leak(form, watts, seconds);
    }
    let summary = production.close().unwrap();
    assert_eq!(summary.events, casos.len());

    // cadeia do binário íntegra (o encoder novo fecha o elo cru)
    let rel = verify_binary(&path).unwrap();
    assert!(rel.chain_ok, "cadeia do caminho direto deve verificar");
    assert_eq!(rel.counts.get("LEAK"), Some(&(casos.len() as u64)));

    // linhas byte a byte idênticas
    let lines = vcad_lines(&path);
    assert_eq!(lines.len(), casos.len());
    for (i, (form, ..)) in casos.iter().enumerate() {
        let esperada = reference.events[i].line();
        assert_eq!(lines[i], esperada, "caso {i}: '{form}'");
    }

    // roundtrip JSONL: o export do binário novo é auditável pelo verificador
    let jsonl = dir.join("caderno.jsonl");
    let n = jsonl_from_binary(&path, &jsonl).unwrap();
    assert_eq!(n, casos.len());
    let rel_jsonl = verify_jsonl(&jsonl).unwrap();
    assert!(
        rel_jsonl.chain_ok,
        "cadeia do JSONL exportado deve verificar"
    );
    // Joules agregados batem com a soma canônica dos casos
    let expected_j: f64 = casos.iter().map(|(_, w, s, _, _)| w * s).sum();
    assert!((rel_jsonl.total_joules - expected_j).abs() < 1e-9);

    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Bateria de cláusulas de corrupção e acessórios (verificação honesta)
// ======================================================================

#[test]
fn summary_acessorios_e_media_sem_formas() {
    let dir = temp_dir("acessorios");
    let path = dir.join("caderno.vcad");
    let mut ledger = ProductionLedger::open(&path).unwrap();
    assert_eq!(ledger.path(), path.as_path());
    assert_eq!(ledger.enqueued(), 0);
    ledger.record("INFO", "único", Json::obj([]));
    let summary = ledger.close().unwrap();
    assert_eq!(summary.events, 1);
    // sem vazamentos registrados, média por forma é 0.0 honesto
    assert_eq!(summary.avg_joules_per_form(), 0.0);
}

#[test]
fn leak_registra_vazamento_e_media_por_forma() {
    let dir = temp_dir("leak");
    let path = dir.join("caderno.vcad");
    let mut ledger = ProductionLedger::open(&path).unwrap();
    ledger.set_time(1, 1.0);
    ledger.leak("FormaVazando", 7.5, 1.0); // 7,5 W por 1 s = 7,5 J
    ledger.leak("FormaVazando", 7.5, 1.0); // segunda partilha, mesma forma
    let summary = ledger.close().unwrap();
    assert_eq!(summary.events, 2);
    assert!(
        (summary.total_joules - 15.0).abs() < 1e-9,
        "{}",
        summary.total_joules
    );
    // joules_per_form agrega POR FORMA: 2 partilhas de 7,5 J na mesma forma
    // ⇒ média por forma com vazamento = 15 J.
    let media = summary.avg_joules_per_form();
    assert!((media - 15.0).abs() < 1e-9, "{media}");
    // LEAK está no log e a cadeia fecha
    let rel = verify_binary(&path).unwrap();
    assert!(rel.chain_ok, "cadeia com LEAK deve fechar");
    assert_eq!(rel.events, 2);
}

#[test]
fn close_com_diretorio_impossivel_reporta_erro_da_thread() {
    // O arquivo só é criado pela thread de gravação: caminho impossível ⇒
    // open devolve Ok (defere) e close reporta o erro real da thread.
    let dir = temp_dir("impossivel");
    let path = dir.join("nao").join("existe").join("caderno.vcad");
    let ledger = ProductionLedger::open(&path).expect("open adia a criação");
    let err = ledger.close().unwrap_err();
    assert!(err.contains(&path.display().to_string()), "{err}");
}

#[test]
fn flush_periodico_em_carga_maior_que_o_lote() {
    let dir = temp_dir("flush");
    let path = dir.join("caderno.vcad");
    let mut ledger = ProductionLedger::open(&path).unwrap();
    for i in 0..600 {
        // FLUSH_EVERY = 256: 600 eventos cruzam dois ciclos de flush.
        ledger.record("INFO", &format!("evento {i}"), Json::obj([]));
    }
    let summary = ledger.close().unwrap();
    assert_eq!(summary.events, 600);
    let rel = verify_binary(&path).unwrap();
    assert!(rel.chain_ok);
    assert_eq!(rel.events, 600);
}

// ── verify/verify_binary: cláusulas de corrupção byte a byte ─────────────

const MAGIC_BYTES: &[u8; 4] = b"VCAD";

#[test]
fn verify_de_arquivo_ausente_e_curto_falham_honesto() {
    let dir = temp_dir("ausente");
    let err = verify(&dir.join("não-existe.vcad")).unwrap_err();
    assert!(err.contains("não-existe.vcad"), "{err}");

    let curto = dir.join("curto.vcad");
    std::fs::write(&curto, b"VC").unwrap(); // 2 bytes < 4 do magic ⇒ JSONL
    let rel = verify(&curto).unwrap(); // linha não-JSON ⇒ cadeia quebrada
    assert!(!rel.chain_ok);
    assert_eq!(rel.events, 0);

    let magic_ruim = dir.join("magic.vcad");
    std::fs::write(&magic_ruim, b"XXXX\x01").unwrap();
    // magic ≠ VCAD ⇒ cai no verificador JSONL ⇒ linha não-JSON quebra a cadeia
    let rel = verify(&magic_ruim).unwrap();
    assert!(!rel.chain_ok);
}

#[test]
fn verify_binary_rejeita_versao_desconhecida() {
    let dir = temp_dir("versao");
    let path = dir.join("v9.vcad");
    let mut bytes = MAGIC_BYTES.to_vec();
    bytes.push(9); // VERSION = 1
    std::fs::write(&path, &bytes).unwrap();
    let err = verify_binary(&path).unwrap_err();
    assert!(err.contains("versão 9"), "{err}");
}

fn frame_valido(head_hex: &str, linha: &str) -> Vec<u8> {
    use vbl_runtime::ledger::sha256_double_hex;
    let mut out = Vec::new();
    out.extend_from_slice(&(linha.len() as u32).to_le_bytes());
    out.extend_from_slice(linha.as_bytes());
    let digest = sha256_double_hex(head_hex.as_bytes(), linha.as_bytes());
    for b in digest.as_bytes().chunks(2) {
        out.push(u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap());
    }
    out
}

#[test]
fn verify_binary_detecta_frame_truncado_e_utf8_invalido() {
    // NOME ÚNICO: temp_dir() remove o diretório no início — reusar um nome
    // de outro teste (rodando em paralelo no MESMO processo/pid) apaga os
    // arquivos do outro e produz falha esporádica de NotFound.
    let dir = temp_dir("truncado-frame");

    // Frame com tamanho maior que o corpo disponível ⇒ chain_ok = false.
    let trunc = dir.join("trunc.vcad");
    let mut bytes = MAGIC_BYTES.to_vec();
    bytes.push(1);
    bytes.extend_from_slice(&(200u32).to_le_bytes()); // promete 200 B…
    bytes.extend_from_slice(b"linha curta"); // …entrega 11
    std::fs::write(&trunc, &bytes).unwrap();
    let rel = verify_binary(&trunc).unwrap();
    assert!(!rel.chain_ok);
    assert_eq!(rel.first_broken, Some(0));

    // Linha com UTF-8 inválido ⇒ erro honesto (não confia em bytes soltos).
    let nao_utf8 = dir.join("naoutf8.vcad");
    let mut bytes = MAGIC_BYTES.to_vec();
    bytes.push(1);
    bytes.extend_from_slice(&4u32.to_le_bytes());
    bytes.extend_from_slice(&[0xFF, 0xFE, 0xFD, 0xFC]);
    bytes.extend_from_slice(&[0u8; 32]);
    std::fs::write(&nao_utf8, &bytes).unwrap();
    assert!(verify_binary(&nao_utf8)
        .unwrap_err()
        .contains("UTF-8 inválido"));
}

#[test]
fn verify_binary_detecta_hash_trocada_e_rodape_mentiroso() {
    let dir = temp_dir("hashrodape");

    // Frame válido + hash ZERADA ⇒ first_broken = 0, mas o parse segue.
    let path = dir.join("hash.vcad");
    let linha = "{\"seq\":0}".to_string();
    let mut bytes = MAGIC_BYTES.to_vec();
    bytes.push(1);
    bytes.extend_from_slice(&(linha.len() as u32).to_le_bytes());
    bytes.extend_from_slice(linha.as_bytes());
    bytes.extend_from_slice(&[0u8; 32]); // hash errada de propósito
    std::fs::write(&path, &bytes).unwrap();
    let rel = verify_binary(&path).unwrap();
    assert!(!rel.chain_ok);
    assert_eq!(rel.first_broken, Some(0));

    // Cadeia ÍNTEGA + rodapé com head diferente ⇒ footer_ok = false.
    let path2 = dir.join("rodape.vcad");
    let mut bytes2 = MAGIC_BYTES.to_vec();
    bytes2.push(1);
    // um frame legítimo, construído com a cabeça inicial real
    bytes2.extend_from_slice(&frame_valido(
        vbl_runtime::ledger::ChainLedger::INITIAL_HEAD,
        "{\"seq\":1}",
    ));
    // rodapé: magic correto, 0 eventos, head de 64 'z' (≠ real)
    bytes2.extend_from_slice(b"VFIM");
    bytes2.extend_from_slice(&0u32.to_le_bytes());
    bytes2.extend_from_slice(&[b'z'; 64]);
    std::fs::write(&path2, &bytes2).unwrap();
    let rel2 = verify_binary(&path2).unwrap();
    assert!(
        !rel2.chain_ok || !rel2.footer_ok,
        "rodapé mentiroso deve falhar"
    );
}

// ── Rodapé, JSONL e conversão: cláusulas restantes da verificação ─────────

fn binario_com_rodape(frames: &[u8], eventos: u32, head_hex: &str) -> Vec<u8> {
    let mut v = MAGIC_BYTES.to_vec();
    v.push(1);
    v.extend_from_slice(frames);
    v.extend_from_slice(b"VFIM");
    v.extend_from_slice(&eventos.to_le_bytes());
    assert_eq!(head_hex.len(), 64);
    v.extend_from_slice(head_hex.as_bytes());
    v
}

#[test]
fn rodape_com_head_nao_utf8_marca_footer_ok_false() {
    let dir = temp_dir("rodape-naoutf8");
    let path = dir.join("r.vcad");
    let mut bytes = binario_com_rodape(&[], 0, &"0".repeat(64));
    // sobrescreve os 64 bytes do head com UTF-8 inválido
    let start = bytes.len() - 64;
    for b in &mut bytes[start..] {
        *b = 0xFF;
    }
    std::fs::write(&path, &bytes).unwrap();
    let rel = verify_binary(&path).unwrap();
    assert!(!rel.footer_ok, "{rel:?}");
}

#[test]
fn rodape_mentiroso_e_detectado_por_head_e_por_eventos() {
    let dir = temp_dir("rodape-mentira");
    // head do rodapé ≠ cabeça real da cadeia
    let p1 = dir.join("head.vcad");
    let real_head_diferente = "a".repeat(64);
    std::fs::write(&p1, binario_com_rodape(&[], 0, &real_head_diferente)).unwrap();
    let rel = verify_binary(&p1).unwrap();
    assert!(!rel.chain_ok || !rel.footer_ok, "{rel:?}");

    // contagem de eventos do rodapé ≠ número real de frames
    let p2 = dir.join("eventos.vcad");
    std::fs::write(&p2, binario_com_rodape(&[], 7, &"0".repeat(64))).unwrap();
    let rel = verify_binary(&p2).unwrap();
    assert!(!rel.chain_ok && !rel.footer_ok, "{rel:?}");
}

#[test]
fn frame_com_hash_cortada_quebra_a_cadeia() {
    let dir = temp_dir("hash-cortada");
    let path = dir.join("h.vcad");
    let mut bytes = MAGIC_BYTES.to_vec();
    bytes.push(1);
    let linha = b"{\"seq\":0}";
    bytes.extend_from_slice(&(linha.len() as u32).to_le_bytes());
    bytes.extend_from_slice(linha);
    bytes.extend_from_slice(&[0xAA; 16]); // hash de 32 esperada; só 16
    std::fs::write(&path, &bytes).unwrap();
    let rel = verify_binary(&path).unwrap();
    assert!(!rel.chain_ok);
    assert_eq!(rel.first_broken, Some(0));
}

#[test]
fn verify_jsonl_exige_kind_e_hash_e_toleram_sem_msg_seq() {
    use vbl_runtime::ledger::sha256_double_hex;
    let dir = temp_dir("jsonl-clausulas");

    // linha sem "kind" ⇒ cadeia quebrada no primeiro evento.
    let sem_kind = dir.join("sem-kind.jsonl");
    std::fs::write(&sem_kind, "{\"seq\":0,\"msg\":\"x\",\"hash\":\"00\"}\n").unwrap();
    let rel = verify_jsonl(&sem_kind).unwrap();
    assert!(!rel.chain_ok);
    assert_eq!(rel.first_broken, Some(0));

    // linha sem "hash" ⇒ idem.
    let sem_hash = dir.join("sem-hash.jsonl");
    std::fs::write(&sem_hash, "{\"seq\":0,\"kind\":\"INFO\",\"msg\":\"x\"}\n").unwrap();
    let rel = verify_jsonl(&sem_hash).unwrap();
    assert!(!rel.chain_ok);

    // sem msg/seq: tolerado (msg vira "", seq assume a ordem) — cadeia fecha
    // com o hash canônico correspondente.
    let linha = "0\u{1f}INFO\u{1f}";
    let hash = sha256_double_hex(
        vbl_runtime::ledger::ChainLedger::INITIAL_HEAD.as_bytes(),
        linha.as_bytes(),
    );
    let ok = dir.join("ok.jsonl");
    std::fs::write(&ok, format!("{{\"kind\":\"INFO\",\"hash\":\"{hash}\"}}\n")).unwrap();
    let rel = verify_jsonl(&ok).unwrap();
    assert!(rel.chain_ok, "{rel:?}");
    assert_eq!(rel.events, 1);
}

#[test]
fn jsonl_from_binary_com_linha_vazia_falha_honesto() {
    let dir = temp_dir("jsonl-vazia");
    let bin = dir.join("b.vcad");
    let jsonl = dir.join("b.jsonl");
    // frame de tamanho 0 (linha vazia): conversão recusa "seq inválido".
    let mut bytes = MAGIC_BYTES.to_vec();
    bytes.push(1);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 32]);
    bytes.extend_from_slice(b"VFIM");
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&[b'0'; 64]);
    std::fs::write(&bin, &bytes).unwrap();
    let err = jsonl_from_binary(&bin, &jsonl).unwrap_err();
    assert!(!err.is_empty(), "{err}");
}

#[test]
fn jsonl_from_binary_converte_ledger_real_e_recusa_nao_vcad() {
    let dir = temp_dir("jsonl-converte");
    // caminho feliz: ledger com ACTUATION (stats v1.1) e um INFO.
    let mut ledger = ProductionLedger::open(dir.join("c.vcad")).unwrap();
    ledger.record("INFO", "olá", Json::obj([("forma", Json::str("A"))]));
    ledger.record(
        "ACTUATION",
        "atuou",
        Json::obj([("ator", Json::str("Fan"))]),
    );
    ledger.close().unwrap();
    let bin = dir.join("c.vcad");
    let jsonl = dir.join("c.jsonl");
    let n = jsonl_from_binary(&bin, &jsonl).unwrap();
    assert_eq!(n, 2, "{n}");
    let rel = verify_jsonl(&jsonl).unwrap();
    assert!(rel.chain_ok, "{rel:?}");
    assert_eq!(rel.counts.get("ACTUATION"), Some(&1));

    // não-vcad ⇒ erro tipado.
    let falso = dir.join("falso.vcad");
    std::fs::write(&falso, b"XXXX\x01").unwrap();
    assert!(jsonl_from_binary(&falso, &dir.join("f.jsonl")).is_err());
}

#[test]
fn jsonl_from_binary_para_em_frame_com_hash_cortada() {
    let dir = temp_dir("jsonl-hash-corta");
    let bin = dir.join("b.vcad");
    let jsonl = dir.join("b.jsonl");
    // frame com linha válida mas hash cortada (16 de 32 bytes), sem rodapé:
    // a conversão para honestamente (era um frame truncado/rodapé).
    let mut bytes = MAGIC_BYTES.to_vec();
    bytes.push(1);
    let linha = "0\u{1f}INFO\u{1f}x".to_string().into_bytes();
    bytes.extend_from_slice(&(linha.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&linha);
    bytes.extend_from_slice(&[0x55; 16]);
    std::fs::write(&bin, &bytes).unwrap();
    let n = jsonl_from_binary(&bin, &jsonl).unwrap();
    assert_eq!(n, 0, "{n}");
}

#[test]
fn verify_jsonl_contabiliza_actuation_e_alertas() {
    let dir = temp_dir("jsonl-actuation");
    let mut ledger = ProductionLedger::open(dir.join("d.vcad")).unwrap();
    ledger.record("INFO", "a", Json::obj([]));
    ledger.record("ACTUATION", "b", Json::obj([("status", Json::str("ok"))]));
    ledger.record("ALERT", "c", Json::obj([]));
    ledger.close().unwrap();
    // acumular_stats (contadores de atuação) vive na verificação BINÁRIA.
    let rel = verify_binary(&dir.join("d.vcad")).unwrap();
    assert!(rel.chain_ok, "{rel:?}");
    assert_eq!(rel.counts.get("ALERT"), Some(&1));
    assert!(rel.actuations >= 1, "{rel:?}");
    let jsonl = dir.join("d.jsonl");
    jsonl_from_binary(&dir.join("d.vcad"), &jsonl).unwrap();
    let rel = verify_jsonl(&jsonl).unwrap();
    assert!(
        rel.chain_ok && rel.counts.get("ACTUATION") == Some(&1),
        "{rel:?}"
    );
}

#[test]
fn json_e_ledger_bordas_de_parse_e_filtros() {
    use vbl_runtime::json::Json;
    use vbl_runtime::ledger::ChainLedger;

    // Default do caderno em cadeia é equivalente a new().
    let mut d = ChainLedger::default();
    d.record("INFO", "olá", Json::obj([("forma", Json::str("A"))]));
    assert_eq!(d.events.len(), 1);

    // Objeto aninhado: array mal-fechado e literal true/false/null.
    assert!(Json::parse("{\"a\":[1,2}").is_none(), "array mal fechado");
    assert!(Json::parse("[true,false,null]").is_some());
    assert!(Json::parse("{\"x\":tru}").is_none(), "literal cortado");
    // escapes completos: \" \\ \/ \n \r \t \b \f \uXXXX.
    let esc = Json::parse("\"a\\b\\n\\r\\t\\f\\u0041\\u00ff\\/\"").expect("escapes");
    match esc {
        Json::Str(s) => {
            assert!(s.contains("a\u{8}\n\r\t\u{c}A\u{ff}/"), "{s:?}");
        }
        other => panic!("esperava string: {other:?}"),
    }
    // escape inválido (\§) ⇒ parse recusa; string não fechada idem.
    assert!(Json::parse("\"\\x\"").is_none());
    assert!(Json::parse("\"sem fim").is_none());
    // objeto e array malformados ⇒ None.
    assert!(Json::parse("{:}").is_none());
    assert!(Json::parse("[,").is_none());
    assert!(Json::parse("{\"k\":1,}").is_none());

    // count_with com extra escalar (não-Objeto) ⇒ filtro devolve falso.
    let mut l = ChainLedger::new();
    l.record("INFO", "x", Json::obj([("k", Json::str("v"))]));
    assert!(l.count_with("INFO", &[("k", Json::str("v"))]));
    // filtro por chave ausente ⇒ falso (braço `_ => false` de field_equal)
    assert!(!l.count_with("INFO", &[("outra", Json::str("v"))]));
    assert!(!l.count_with("OUTRO_KIND", &[("k", Json::str("v"))]));
}

#[test]
fn verify_binary_em_arquivo_ausente_falha_honesto() {
    let dir = temp_dir("binario-ausente");
    let err = verify_binary(&dir.join("fantasma.vcad")).unwrap_err();
    assert!(err.contains("fantasma"), "{err}");
}
