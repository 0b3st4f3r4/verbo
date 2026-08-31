//! Caderno de produção (Etapa 4 — PLAN §4.1; AGENTS §1.4): gravação
//! assíncrona, formato binário `.vcad`, timestamps do relógio virtual,
//! atuação com custo estimado, verificação externa e estresse 10k formas.

use std::io::Write;
use std::path::PathBuf;
use vbl_runtime::caderno::{Atuacao, Caderno, ChainCaderno};
use vbl_runtime::caderno_producao::{
    jsonl_de_binario, verificar, verificar_binario, verificar_jsonl, CadernoProducao,
};
use vbl_runtime::fxp::Value;
use vbl_runtime::json::Json;
use vbl_runtime::{carregar, Engine, FxpSimulator};

fn dir_tempo(nome: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vbl-caderno-{nome}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ======================================================================
// Gravação assíncrona + integridade (cadeia SHA-256)
// ======================================================================

#[test]
fn gravacao_assincrona_produz_cadeia_integra() {
    let dir = dir_tempo("integra");
    let caminho = dir.join("caderno.vcad");
    let mut caderno = CadernoProducao::abrir(&caminho).unwrap();
    caderno.definir_tempo(1, 1.0);
    caderno.record("INFO", "primeiro evento", Json::obj([("forma", Json::str("A"))]));
    caderno.definir_tempo(2, 2.0);
    caderno.record("INFO", "segundo evento", Json::obj([("forma", Json::str("B"))]));
    let resumo = caderno.fechar().unwrap();
    assert_eq!(resumo.eventos, 2);
    assert!(resumo.bytes > 5 + 4);
    assert_ne!(resumo.chain_head, ChainCaderno::HEAD_INICIAL);

    let rel = verificar_binario(&caminho).unwrap();
    assert_eq!(rel.eventos, 2);
    assert!(rel.cadeia_ok, "cadeia deve estar íntegra");
    assert!(rel.primeiro_quebrado.is_none());
    assert!(rel.rodape_ok, "rodapé presente e coerente");
    assert_eq!(rel.chain_head, resumo.chain_head);
    assert_eq!(rel.contagens.get("INFO"), Some(&2));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn adulteracao_binaria_quebra_a_cadeia() {
    let dir = dir_tempo("adulterado");
    let caminho = dir.join("caderno.vcad");
    let mut caderno = CadernoProducao::abrir(&caminho).unwrap();
    caderno.record("INFO", "evento original", Json::obj([]));
    caderno.record("INFO", "outro evento", Json::obj([]));
    caderno.fechar().unwrap();

    let mut dados = std::fs::read(&caminho).unwrap();
    // adulteração retroativa no primeiro frame (payload: header 5 bytes + len 4)
    let alvo = 9 + 4; // dentro do campo "kind"/"msg" da linha 0
    dados[alvo] = b'X';
    std::fs::write(&caminho, &dados).unwrap();

    let rel = verificar_binario(&caminho).unwrap();
    assert!(!rel.cadeia_ok, "adulteração deve quebrar a cadeia");
    assert_eq!(rel.primeiro_quebrado, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jsonl_do_binario_reproduz_a_cadeia() {
    let dir = dir_tempo("jsonl");
    let binario = dir.join("caderno.vcad");
    let jsonl = dir.join("caderno.jsonl");
    let mut caderno = CadernoProducao::abrir(&binario).unwrap();
    caderno.definir_tempo(3, 3.0);
    caderno.record("INFO", "com timestamps", Json::obj([("forma", Json::str("A"))]));
    caderno.fechar().unwrap();

    let n = jsonl_de_binario(&binario, &jsonl).unwrap();
    assert_eq!(n, 1);
    let rel_bin = verificar_binario(&binario).unwrap();
    let rel_jsonl = verificar_jsonl(&jsonl).unwrap();
    assert!(rel_jsonl.cadeia_ok);
    assert_eq!(rel_bin.chain_head, rel_jsonl.chain_head, "mesma cadeia nos dois formatos");

    // o JSONL exporta os timestamps no nível superior (AGENTS §1.4)
    let texto = std::fs::read_to_string(&jsonl).unwrap();
    assert!(texto.contains("\"tick\":3"), "tick do relógio virtual no JSONL: {texto}");
    assert!(texto.contains("\"t\":3"), "t (segundos virtuais) no JSONL: {texto}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jsonl_adulterado_quebra_a_cadeia() {
    let dir = dir_tempo("jsonl-adulterado");
    let jsonl = dir.join("caderno.jsonl");
    let mut caderno = ChainCaderno::new();
    caderno.record("INFO", "linha original", Json::obj([]));
    caderno.export_jsonl(&jsonl).unwrap();

    let mut texto = std::fs::read_to_string(&jsonl).unwrap();
    texto = texto.replace("linha original", "linha FORJADA");
    std::fs::write(&jsonl, &texto).unwrap();

    let rel = verificar_jsonl(&jsonl).unwrap();
    assert!(!rel.cadeia_ok);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deteccao_automatica_de_formato() {
    let dir = dir_tempo("deteccao");
    let binario = dir.join("a.vcad");
    let jsonl = dir.join("a.jsonl");
    let mut caderno = CadernoProducao::abrir(&binario).unwrap();
    caderno.record("INFO", "evento", Json::obj([]));
    caderno.fechar().unwrap();
    jsonl_de_binario(&binario, &jsonl).unwrap();
    // `verificar` escolhe pelo magic: binário e JSONL íntegros, mesma cadeia
    let a = verificar(&binario).unwrap();
    let b = verificar(&jsonl).unwrap();
    assert!(a.cadeia_ok && b.cadeia_ok);
    assert_eq!(a.chain_head, b.chain_head);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drop_sem_fechar_grava_rodape_valido() {
    let dir = dir_tempo("drop");
    let caminho = dir.join("caderno.vcad");
    {
        let mut caderno = CadernoProducao::abrir(&caminho).unwrap();
        caderno.record("INFO", "antes do drop", Json::obj([]));
    } // Drop: encerra a thread e fecha o arquivo
    let rel = verificar_binario(&caminho).unwrap();
    assert!(rel.cadeia_ok);
    assert_eq!(rel.eventos, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Agregados (Joules por forma — AGENTS §1.4)
// ======================================================================

#[test]
fn agregados_de_joules_por_forma() {
    let dir = dir_tempo("joules");
    let caminho = dir.join("caderno.vcad");
    let mut caderno = CadernoProducao::abrir(&caminho).unwrap();
    caderno.leak("A", 30.0, 1.0);
    caderno.leak("A", 10.0, 1.0);
    caderno.leak("B", 60.0, 2.0);
    let resumo = caderno.fechar().unwrap();
    assert!((resumo.joules_totais - 160.0).abs() < 1e-9);
    assert!((resumo.joules_por_forma["A"] - 40.0).abs() < 1e-9);
    assert!((resumo.joules_por_forma["B"] - 120.0).abs() < 1e-9);
    assert!((resumo.media_joules_por_forma() - 80.0).abs() < 1e-9);
    // os eventos VAZAMENTO também estão no arquivo, com os mesmos Joules
    let rel = verificar_binario(&caminho).unwrap();
    assert!((rel.joules_totais - 160.0).abs() < 1e-9);
    assert_eq!(rel.contagens.get("VAZAMENTO"), Some(&3));
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Atuação com valor aplicado, latência e custo energético (PLAN §4.1)
// ======================================================================

#[test]
fn atuacao_registra_aplicado_latencia_e_custo() {
    let dir = dir_tempo("atuacao");
    let caminho = dir.join("caderno.vcad");
    let mut caderno = CadernoProducao::abrir(&caminho).unwrap();
    caderno.definir_potencia(100.0); // potência do tick (W)
    caderno.actuator_action_detalhada(Atuacao {
        ator: "CpuPowerCap".into(),
        solicitado: Value::Num(50.0),
        aplicado: Some(Value::Num(50.0)),
        latencia_us: Some(250),
        custo_joules: None, // estimado: 100 W × 250 µs = 0.025 J
        sucesso: true,
    });
    caderno.actuator_action_detalhada(Atuacao {
        ator: "Ventoinha".into(),
        solicitado: Value::Num(255.0),
        aplicado: None,
        latencia_us: None,
        custo_joules: None,
        sucesso: false,
    });
    caderno.fechar().unwrap();

    let rel = verificar_binario(&caminho).unwrap();
    assert_eq!(rel.atuacoes, 2);
    assert_eq!(rel.atuacoes_ok, 1);
    // confere os campos no JSONL exportado
    let jsonl = dir.join("caderno.jsonl");
    jsonl_de_binario(&caminho, &jsonl).unwrap();
    let texto = std::fs::read_to_string(&jsonl).unwrap();
    let linha_atuacao = texto.lines().next().unwrap();
    assert!(linha_atuacao.contains("\"aplicado\":50"), "{linha_atuacao}");
    assert!(linha_atuacao.contains("\"latencia_us\":250"), "{linha_atuacao}");
    assert!(
        linha_atuacao.contains("\"custo_estimado_joules\":0.025"),
        "custo = potência × latência: {linha_atuacao}"
    );
    // falha não tem valor aplicado nem custo (nada foi aplicado)
    let linha_falha = texto.lines().nth(1).unwrap();
    assert!(!linha_falha.contains("\"aplicado\""), "{linha_falha}");
    assert!(!linha_falha.contains("\"custo_estimado_joules\""), "{linha_falha}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Engine + Caderno de produção (integração)
// ======================================================================

#[test]
fn engine_com_caderno_de_producao_carimba_relogio_virtual() {
    let dir = dir_tempo("engine-relogio");
    let caminho = dir.join("caderno.vcad");
    let sim = FxpSimulator::novo();
    let mut engine = Engine::com_caderno(
        sim,
        1.0,
        dir.join("persistencia"),
        CadernoProducao::abrir(&caminho).unwrap(),
    );
    let (programa, diags) = vbl_lang::parse("event X { value: \"v\", horizon: 3s }");
    assert!(diags.items.is_empty());
    let _interp = carregar(&mut engine, &programa);
    engine.tick();
    drop(engine); // fecha a thread de gravação

    let jsonl = dir.join("caderno.jsonl");
    jsonl_de_binario(&caminho, &jsonl).unwrap();
    let texto = std::fs::read_to_string(&jsonl).unwrap();
    // eventos do tick 1 carregam tick=1 e t=1 (relógio virtual — AGENTS §1.4)
    assert!(texto.contains("\"tick\":1"), "{texto}");
    assert!(texto.contains("\"t\":1"), "{texto}");
    let rel = verificar_binario(&caminho).unwrap();
    assert!(rel.cadeia_ok);
    assert!(rel.eventos > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Estresse: 10.000 formas ativas (AGENTS §1.4/§1.5; PLAN §4.2)
// ======================================================================

#[test]
fn estresse_10_mil_formas_todos_os_eventos_gravados() {
    const FORMAS: usize = 10_000;
    const TICKS: usize = 5;
    let dir = dir_tempo("estresse");
    let caminho = dir.join("caderno.vcad");
    let sim = FxpSimulator::novo();
    let mut engine = Engine::com_caderno(
        sim,
        1.0,
        dir.join("persistencia"),
        CadernoProducao::abrir(&caminho).unwrap(),
    );
    let mut fonte = String::new();
    for i in 0..FORMAS {
        fonte.push_str(&format!("event F{i} {{ value: \"v{i}\", horizon: 1000000s }}\n"));
    }
    let (programa, diags) = vbl_lang::parse(&fonte);
    assert!(!diags.has_errors());
    let _interp = carregar(&mut engine, &programa);
    for _ in 0..TICKS {
        engine.tick();
    }
    let enfileirados = engine.caderno.enfileirados();
    // ≥ 1 VAZAMENTO por forma por tick (10k × 5 = 50k) + registros de carga
    assert!(enfileirados >= FORMAS * TICKS);
    drop(engine);

    let rel = verificar_binario(&caminho).unwrap();
    assert_eq!(rel.eventos, enfileirados, "cobertura de eventos: 100%");
    assert!(rel.cadeia_ok, "cadeia íntegra sob carga máxima");
    assert!(rel.joules_totais > 0.0);
    // robustez: nenhum evento perdido na fila (AGENTS §1.4 — 99,99%+)
    assert_eq!(
        rel.contagens.get("VAZAMENTO"),
        Some(&((FORMAS * TICKS) as u64))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ======================================================================
// Parser JSON mínimo (base do verificador)
// ======================================================================

#[test]
fn parser_json_roundtrip_deterministico() {
    let original = Json::obj([
        ("seq", Json::num(7.0)),
        ("kind", Json::str("ATUACAO")),
        ("msg", Json::str("Ator 'Ventoinha' <- 200 (sucesso)")),
        ("valor", Json::num(200.0)),
        ("decimal", Json::num(0.5)),
        ("negativo", Json::num(-3.25)),
        ("lista", Json::Arr(vec![Json::str("a"), Json::boolean(true), Json::Nulo])),
        ("objeto", Json::obj([("chave", Json::str("valor \"escapado\""))])),
    ]);
    let texto = original.serializar();
    let lido = Json::analisar(&texto).expect("parser deve ler o que o serializador escreve");
    assert_eq!(lido.serializar(), texto, "roundtrip estável");
    assert!(Json::analisar(&format!("{texto} lixo")).is_none());
    assert!(Json::analisar("{\"a\":}").is_none());
}

/// Escrita direta de bytes (uso do testes de tolerância a truncagem).
#[allow(dead_code)]
fn escrever_bytes(caminho: &std::path::Path, dados: &[u8]) {
    let mut f = std::fs::File::create(caminho).unwrap();
    f.write_all(dados).unwrap();
}

#[test]
fn arquivo_truncado_e_rejeitado_com_erro_claro() {
    let dir = dir_tempo("truncado");
    let caminho = dir.join("caderno.vcad");
    let mut caderno = CadernoProducao::abrir(&caminho).unwrap();
    caderno.record("INFO", "evento", Json::obj([]));
    caderno.fechar().unwrap();
    // trunca no meio do primeiro frame (sem hash completo)
    let dados = std::fs::read(&caminho).unwrap();
    std::fs::write(&caminho, &dados[..dados.len() - 40]).unwrap();
    let rel = verificar_binario(&caminho).unwrap();
    assert!(!rel.cadeia_ok, "truncagem deve quebrar a cadeia");
    let _ = std::fs::remove_dir_all(&dir);
}
