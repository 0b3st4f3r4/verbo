//! Matriz de cláusulas de erro do parser (FORMAL §3) — os caminhos que a
//! `productions_matrix` não alcança com programas válidos.
//!
//! Cada linha da tabela é uma cláusula: o programa que a dispara e o código
//! estável do diagnóstico (mesmos códigos reancorados da Etapa 1/2). A
//! recuperação do parser (consumir até `,` ou `}`) também é exercida.

use vbl_lang::parse;

fn codes(text: &str) -> Vec<String> {
    let (_, d) = parse(text);
    d.items.iter().map(|i| i.code.clone()).collect()
}

fn contem(text: &str, esperados: &[&str]) {
    let obtidos = codes(text);
    for e in esperados {
        assert!(
            obtidos.iter().any(|c| c == e),
            "esperado diagnóstico [{e}] em:\n{text}\nobtidos: {obtidos:?}"
        );
    }
}

#[test]
fn matriz_de_clausulas_de_erro() {
    let casos: &[(&str, &[&str])] = &[
        // ── estrutura de forma ────────────────────────────────────────────
        // fim inesperado em expect_punct (sem '{' após o nome)
        ("event A", &["estrutura_forma"]),
        // fim inesperado antes do identificador da forma
        ("event", &["estrutura_forma"]),
        // token não-identificador como nome de atributo no corpo
        ("event A { value: 1, horizon: 5s, 42 }", &["estrutura_forma"]),
        // atributo sem ':' — espera-se dois-pontos
        ("event A { value 1, horizon: 5s }", &["estrutura_forma", "value_obrigatorio"]),
        // recuperação consome até vírgula/fecha, com parênteses aninhados
        (
            "event A { estranho: valor(outro, mais), value: 1, horizon: 5s }",
            &["atributo_desconhecido"],
        ),
        // ── atributos opcionais com valor de tipo errado ──────────────────
        (
            "event A { value: 1, horizon: 5s, source_path: 42 }",
            &["estrutura_forma"],
        ),
        (
            "nonequilibrium T { value: 1, horizon: 5s, maintenance_deadline: 3s, exchange_mode: 42 }",
            &["estrutura_forma"],
        ),
        (
            "equilibrium E { value: 1, horizon: 5s, cost_bytes: texto }",
            &["estrutura_forma"],
        ),
        (
            "equilibrium E { value: 1, horizon: 5s, cost_bytes:",
            &["estrutura_forma", "bloco_nao_fechado"],
        ),
        (
            "event A { value: 1, horizon: 5s, currency: 42 }",
            &["estrutura_forma"],
        ),
        (
            "event A { value: 1, horizon: 5s, classification: 42 }",
            &["estrutura_forma"],
        ),
        // ── expression e duration ─────────────────────────────────────────
        // expression decimal é número (cobre o braço Decimal)
        ("event A { value: 2.5, horizon: 5s }", &[]),
        // expression com token inválido
        ("event A { value: %, horizon: 5s }", &["estrutura_forma"]),
        // duração sem número
        ("event A { value: 1, horizon: s }", &["duracao_invalida"]),
        // duração sem unidade e fim de arquivo
        ("event A { value: 1, horizon:", &["duracao_invalida", "bloco_nao_fechado"]),
        // duração sem unidade (número solto)
        ("event A { value: 1, horizon: 5 }", &["duracao_invalida"]),
        // ── review ────────────────────────────────────────────────────────
        // nome da review não-identificador / ausente
        ("review 42 { when cpu_temp > 1 -> dissolve }", &["estrutura_review"]),
        ("review", &["estrutura_review"]),
        // review sem '{'
        ("review A when cpu_temp > 1 -> dissolve }", &["estrutura_review"]),
        // review sem '}'
        ("review A { when cpu_temp > 1 -> dissolve", &["bloco_nao_fechado"]),
        // regra sem sensor / sem operador / sem threshold (fim de arquivo)
        ("review A { when", &["regra_mal_formada"]),
        ("review A { when cpu_temp", &["operador_invalido"]),
        ("review A { when cpu_temp >", &["regra_mal_formada"]),
        // ator do act mal formado / ausente (dentro de review)
        ("review A { when cpu_temp > 1 -> act(42, 1) }", &["regra_mal_formada"]),
        ("review A { when cpu_temp > 1 -> act(", &["regra_mal_formada"]),
        // action_list com token não-ação / vazia
        ("review A { when cpu_temp > 1 -> 42 }", &["acao_desconhecida"]),
        ("review A { when cpu_temp > 1 ->", &["acao_desconhecida"]),
        // vírgula final na action_list antes de '}'
        (
            "event A { value: 1, horizon: 5s }\nreview A { when cpu_temp > 1 -> dissolve, }",
            &["virgula_final"],
        ),
        // ── main ──────────────────────────────────────────────────────────
        // main sem '{'
        ("event A { value: 1, horizon: 5s }\nmain keep(A) }", &["estrutura_main"]),
        // main sem '}'
        (
            "event A { value: 1, horizon: 5s }\nmain { keep(A)",
            &["bloco_nao_fechado"],
        ),
        // keep com forma não-identificador / ausente
        (
            "event A { value: 1, horizon: 5s }\nmain { keep(42) }",
            &["estrutura_main"],
        ),
        (
            "event A { value: 1, horizon: 5s }\nmain { keep(",
            &["estrutura_main"],
        ),
        // act com ator não-identificador / ausente
        (
            "event A { value: 1, horizon: 5s }\nmain { act(42, 1) }",
            &["estrutura_main"],
        ),
        (
            "event A { value: 1, horizon: 5s }\nmain { act(",
            &["estrutura_main"],
        ),
        // every sem '}'
        (
            "event A { value: 1, horizon: 5s }\nmain { every 2s { keep(A)",
            &["bloco_nao_fechado"],
        ),
        // statement com token não-identificador
        (
            "event A { value: 1, horizon: 5s }\nmain { 42 }",
            &["statement_desconhecido"],
        ),
        // ── main_deve_ser_ultimo: review após o bloco main ────────────────
        (
            "event A { value: 1, horizon: 5s }\nmain { }\nreview A { when cpu_temp > 1 -> dissolve }",
            &["main_deve_ser_ultimo"],
        ),
    ];
    for (programa, esperados) in casos {
        contem(programa, esperados);
    }
}

#[test]
fn expression_decimal_e_horizon_decimal_sao_validos() {
    // braço Decimal da expression (linha 530): valor fracionário direto
    let (program, d) = parse("event A { value: 2.5, horizon: 0.5s }");
    assert!(!d.has_errors(), "{d}");
    let Declaration::Form(f) = &program.decls[0] else {
        panic!("esperado forma");
    };
    assert_eq!(f.value.kind, ExprKind::Num(2.5));
    assert_eq!(f.horizon.seconds(), 0.5);
}

use vbl_lang::{Declaration, ExprKind};
