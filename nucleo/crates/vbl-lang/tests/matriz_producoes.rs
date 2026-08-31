//! Matriz de rastreabilidade do parser — produções EBNF × testes.
//!
//! Cada teste referencia explicitamente a produção da FORMAL §3 que cobre
//! (ver `docs/ETAPA-2-MATRIZ-RASTREABILIDADE.md`): 100% das produções e
//! ≥ 95% das notas semânticas com ≥ 1 teste (critério "Done" da Etapa 2,
//! AGENTS.md §2.2).

use vbl_lang::{
    Action, Conjugation, CmpOp, Conjugation as Conj, ExprKind, PhysicalUnit, TimeUnit,
};
use vbl_lang::parse;

fn erros(texto: &str) -> Vec<String> {
    let (_, d) = parse(texto);
    d.items.iter().map(|i| i.code.clone()).collect()
}

fn sem_erros(texto: &str) {
    let (_, d) = parse(texto);
    assert!(!d.has_errors(), "esperado programa válido, diagnósticos:\n{d}");
}

fn contem(texto: &str, codigo: &str) -> bool {
    erros(texto).iter().any(|c| c == codigo)
}

// ======================================================================
// Produção: program = { form_declaration | review_declaration } [ main_block ]
// ======================================================================
#[test]
fn producao_programa_formas_reviews_e_main() {
    sem_erros(
        "event A { value: \"v\", horizon: 3s }\n\
         review A { when cpu_temp > 90°C -> dissolve }\n\
         main { every 1s { keep(A) } }",
    );
}

#[test]
fn producao_programa_ordem_livre_e_main_opcional() {
    // main ausente; review antes da forma também é aceito (a ligação é
    // resolvida no conjunto de declarações)
    sem_erros("review A { when cpu_temp > 90°C -> dissolve }\nevent A { value: \"v\", horizon: 3s }");
    sem_erros("event A { value: \"v\", horizon: 3s }");
}

#[test]
fn producao_programa_main_deve_ser_ultimo() {
    assert!(contem(
        "main { }\nevent A { value: \"v\", horizon: 3s }",
        "main_deve_ser_ultimo"
    ));
}

#[test]
fn producao_programa_main_duplicado_rejeitado() {
    assert!(contem("main { }\nmain { }", "main_duplicado"));
}

#[test]
fn producao_programa_topo_invalido() {
    assert!(contem("42", "topo_invalido"));
    assert!(contem("foo X { value: \"v\", horizon: 3s }", "topo_invalido"));
}

// ======================================================================
// Produção: form_declaration / conjugation_kw
// ======================================================================
#[test]
fn producao_conjugation_kw_tres_variantes() {
    for (conj_txt, esperada) in [
        ("event", Conj::Event),
        ("equilibrium", Conj::Equilibrium),
        ("nonequilibrium", Conj::Nonequilibrium),
    ] {
        let fonte = match esperada {
            Conj::Nonequilibrium => {
                format!("{conj_txt} X {{ value: \"v\", horizon: 3s, maintenance_deadline: 2s }}")
            }
            _ => format!("{conj_txt} X {{ value: \"v\", horizon: 3s }}"),
        };
        let (p, d) = parse(&fonte);
        assert!(!d.has_errors(), "{conj_txt}: {d}");
        let f = p.forms().next().unwrap();
        assert_eq!(f.conjugation, esperada);
    }
}

#[test]
fn producao_form_declaration_estrutura() {
    // sem identificador / sem '{' / sem '}'
    assert!(contem("event { value: \"v\", horizon: 3s }", "estrutura_forma"));
    assert!(contem("event X value: \"v\" }", "estrutura_forma"));
    assert!(contem("event X { value: \"v\", horizon: 3s", "bloco_nao_fechado"));
}

#[test]
fn producao_form_declaration_duplicada() {
    assert!(contem(
        "event X { value: \"v\", horizon: 3s }\nevent X { value: \"w\", horizon: 4s }",
        "forma_duplicada"
    ));
}

// ======================================================================
// Produção: form_body + cláusulas de Lei 1 (value/horizon)
// ======================================================================
#[test]
fn producao_form_body_value_e_horizon_obrigatorios_e_nesta_ordem() {
    assert!(contem("event X { horizon: 3s }", "value_obrigatorio"));
    assert!(contem("event X { value: \"v\" }", "horizon_obrigatorio"));
    assert!(contem("event X { horizon: 3s, value: \"v\" }", "ordem_value_horizon"));
}

#[test]
fn nota_lei1_value_e_opaco_ao_runtime() {
    // expression aceita string, número e identificador (nota FORMAL §3)
    let (p, d) = parse("event X { value: \"s\", horizon: 3s }");
    assert!(!d.has_errors());
    assert_eq!(p.forms().next().unwrap().value.kind, ExprKind::Str("s".into()));

    let (p, d) = parse("event X { value: 42, horizon: 3s }");
    assert!(!d.has_errors());
    assert_eq!(p.forms().next().unwrap().value.kind, ExprKind::Num(42.0));

    let (p, d) = parse("event X { value: identificador, horizon: 3s }");
    assert!(!d.has_errors());
    assert_eq!(
        p.forms().next().unwrap().value.kind,
        ExprKind::Ident("identificador".into())
    );
}

#[test]
fn producao_form_body_virgula_final_rejeitada() {
    assert!(contem("event X { value: \"v\", horizon: 3s, }", "virgula_final"));
    assert!(contem(
        "main { every 1s { keep(X) }, }",
        "virgula_final"
    ));
}

// ======================================================================
// Produção: optional_attribute (6 variantes)
// ======================================================================
#[test]
fn producao_optional_attribute_source_path() {
    let (p, d) = parse(
        "nonequilibrium X { value: \"v\", horizon: 3s, source_path: \"cpu_temp\", maintenance_deadline: 2s }",
    );
    assert!(!d.has_errors(), "{d}");
    assert_eq!(p.forms().next().unwrap().attrs.source_path.as_deref(), Some("cpu_temp"));
}

#[test]
fn nota_source_path_exclusivamente_simbolico() {
    // caminhos de SO não são source_path válidos (FORMAL §3, nota)
    assert!(contem(
        "nonequilibrium X { value: \"v\", horizon: 3s, source_path: \"/sys/class/thermal/temp\", maintenance_deadline: 2s }",
        "source_path_nao_simbolico"
    ));
    assert!(contem(
        "nonequilibrium X { value: \"v\", horizon: 3s, source_path: \"./local\", maintenance_deadline: 2s }",
        "source_path_nao_simbolico"
    ));
}

#[test]
fn producao_optional_attribute_maintenance_deadline() {
    let (p, d) = parse(
        "nonequilibrium X { value: \"v\", horizon: 3s, maintenance_deadline: 2.5s }",
    );
    assert!(!d.has_errors(), "{d}");
    let f = p.forms().next().unwrap();
    assert_eq!(f.attrs.maintenance_deadline.unwrap().segundos(), 2.5);
}

#[test]
fn nota_maintenance_deadline_obrigatorio_em_nonequilibrium() {
    assert!(contem(
        "nonequilibrium X { value: \"v\", horizon: 3s }",
        "maintenance_deadline_ausente"
    ));
    // e proibido nas demais conjugações
    assert!(contem(
        "event X { value: \"v\", horizon: 3s, maintenance_deadline: 2s }",
        "atributo_nao_aplicavel"
    ));
}

#[test]
fn producao_optional_attribute_exchange_mode() {
    let (p, d) = parse(
        "nonequilibrium X { value: \"v\", horizon: 3s, maintenance_deadline: 2s, exchange_mode: \"extraction\" }",
    );
    assert!(!d.has_errors(), "{d}");
    assert_eq!(
        p.forms().next().unwrap().attrs.exchange_mode.as_deref(),
        Some("extraction")
    );
}

#[test]
fn nota_exchange_mode_apenas_nonequilibrium() {
    assert!(contem(
        "equilibrium X { value: \"v\", horizon: 3s, exchange_mode: \"cooperation\" }",
        "atributo_nao_aplicavel"
    ));
}

#[test]
fn producao_optional_attribute_cost_bytes() {
    let (p, d) = parse("equilibrium X { value: \"v\", horizon: 3s, cost_bytes: 4096 }");
    assert!(!d.has_errors(), "{d}");
    assert_eq!(p.forms().next().unwrap().attrs.cost_bytes, Some(4096));
    // decimal é rejeitado (integer na EBNF)
    assert!(contem(
        "equilibrium X { value: \"v\", horizon: 3s, cost_bytes: 4.5 }",
        "cost_bytes_inteiro"
    ));
}

#[test]
fn nota_cost_bytes_apenas_equilibrium() {
    assert!(contem(
        "event X { value: \"v\", horizon: 3s, cost_bytes: 16 }",
        "atributo_nao_aplicavel"
    ));
}

#[test]
fn producao_optional_attribute_currency() {
    let (p, d) = parse("event X { value: \"v\", horizon: 3s, currency: \"CpuCycles\" }");
    assert!(!d.has_errors(), "{d}");
    assert_eq!(p.forms().next().unwrap().attrs.currency.as_deref(), Some("CpuCycles"));
}

#[test]
fn nota_currency_herda_padrao_da_conjugacao() {
    // padrões canônicos: CpuCycles/DiskBytes/PowerWatts (FORMAL §3)
    assert_eq!(Conjugation::Event.currency_padrao(), "CpuCycles");
    assert_eq!(Conjugation::Equilibrium.currency_padrao(), "DiskBytes");
    assert_eq!(Conjugation::Nonequilibrium.currency_padrao(), "PowerWatts");
}

#[test]
fn producao_optional_attribute_classification() {
    let (p, d) = parse(
        "event X { value: \"v\", horizon: 3s, classification: \"Transiente\" }",
    );
    assert!(!d.has_errors(), "{d}");
    assert_eq!(
        p.forms().next().unwrap().attrs.classification.as_deref(),
        Some("Transiente")
    );
}

#[test]
fn producao_optional_attribute_desconhecido_e_duplicado() {
    assert!(contem("event X { value: \"v\", horizon: 3s, foo: 1 }", "atributo_desconhecido"));
    assert!(contem(
        "event X { value: \"v\", horizon: 3s, horizon: 4s }",
        "atributo_duplicado"
    ));
}

// ======================================================================
// Produção: review_declaration / review_rule / sensor_ref / comparison_op
// ======================================================================
#[test]
fn producao_review_declaration_com_regras() {
    let (p, d) = parse(
        "event A { value: \"v\", horizon: 3s }\n\
         review A { when cpu_temp > 90°C -> dissolve,\n\
                    when cpu_temp < 10°C -> notify_shutdown }",
    );
    assert!(!d.has_errors(), "{d}");
    assert_eq!(p.reviews().next().unwrap().rules.len(), 2);
}

#[test]
fn nota_review_orfa_e_duplicada_sao_erros_de_compilacao() {
    assert!(contem(
        "event X { value: \"v\", horizon: 3s }\nreview Y { when cpu_temp > 90°C -> dissolve }",
        "review_orfa"
    ));
    assert!(contem(
        "event X { value: \"v\", horizon: 3s }\n\
         review X { when cpu_temp > 90°C -> dissolve }\n\
         review X { when cpu_temp < 10°C -> dissolve }",
        "review_duplicada"
    ));
}

#[test]
fn producao_sensor_ref_identificador_ou_string() {
    let (p, d) = parse(
        "event A { value: \"v\", horizon: 3s }\n\
         review A { when \"cpu_temp\" > 90°C -> dissolve }",
    );
    assert!(!d.has_errors(), "{d}");
    assert_eq!(p.reviews().next().unwrap().rules[0].sensor.nome, "cpu_temp");
    // sem sensor → erro
    assert!(contem(
        "event A { value: \"v\", horizon: 3s }\nreview A { when > 90°C -> dissolve }",
        "regra_mal_formada"
    ));
}

#[test]
fn producao_comparison_op_seis_operadores() {
    for (op, esperado) in [
        ("<", CmpOp::Lt),
        (">", CmpOp::Gt),
        ("<=", CmpOp::Le),
        (">=", CmpOp::Ge),
        ("==", CmpOp::Eq),
        ("!=", CmpOp::Ne),
    ] {
        let fonte = format!(
            "event A {{ value: \"v\", horizon: 3s }}\nreview A {{ when cpu_temp {op} 90°C -> dissolve }}"
        );
        let (p, d) = parse(&fonte);
        assert!(!d.has_errors(), "op {op}: {d}");
        assert_eq!(p.reviews().next().unwrap().rules[0].op, esperado);
    }
    // operadores inválidos
    assert!(contem(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp = 90°C -> dissolve }",
        "lexema_invalido"
    ));
    assert!(contem(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp ~ 90°C -> dissolve }",
        "operador_invalido"
    ));
}

// ======================================================================
// Produção: threshold / percentage / physical_quantity
// ======================================================================
#[test]
fn producao_threshold_numero_porcentagem_e_grandeza_fisica() {
    let casos = [
        ("when cpu_temp > 90 -> None", 90.0, None),
        ("when attention < 30% -> Some(PhysicalUnit::Percent)", 30.0, Some(PhysicalUnit::Percent)),
        ("when cpu_temp > 85°C -> Some(PhysicalUnit::DegC)", 85.0, Some(PhysicalUnit::DegC)),
        ("when cpu_power >= 150W -> Some(PhysicalUnit::W)", 150.0, Some(PhysicalUnit::W)),
    ];
    for (regra, valor, unidade) in casos {
        let regra = regra
            .replace("-> None", "-> dissolve")
            .replace("-> Some(PhysicalUnit::Percent)", "-> dissolve")
            .replace("-> Some(PhysicalUnit::DegC)", "-> dissolve")
            .replace("-> Some(PhysicalUnit::W)", "-> dissolve");
        let fonte = format!(
            "event A {{ value: \"v\", horizon: 3s }}\nreview A {{ {regra} }}"
        );
        let (p, d) = parse(&fonte);
        assert!(!d.has_errors(), "regra `{regra}`: {d}");
        let t = &p.reviews().next().unwrap().rules[0].threshold;
        assert_eq!(t.valor, valor);
        assert_eq!(t.unit, unidade);
    }
}

#[test]
fn nota_threshold_unidade_e_capturada_para_validacao_no_registro() {
    // A unidade é preservada na AST para validação contra a grandeza do
    // sensor no registro do FXP (FORMAL §3, nota; loader valida em runtime).
    let (p, d) = parse(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 90°C -> dissolve }",
    );
    assert!(!d.has_errors());
    assert_eq!(p.reviews().next().unwrap().rules[0].threshold.unit, Some(PhysicalUnit::DegC));
    // threshold não-numérico é rejeitado
    assert!(contem(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > alto -> dissolve }",
        "regra_mal_formada"
    ));
}

// ======================================================================
// Produção: action_list / action (6 variantes)
// ======================================================================
#[test]
fn producao_action_seis_variantes_na_ordem_declarada() {
    let fonte = "\
event A { value: \"v\", horizon: 30s }\n\
review A { when cpu_temp > 10°C -> dissolve }\n\
review X { when cpu_temp > 10°C -> dissolve }";
    let _ = fonte; // (cobertura simples abaixo)
    let fonte = "\
nonequilibrium A { value: \"v\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
review A { when cpu_temp > 10°C -> notify_shutdown,\n\
           when cpu_temp < 5°C -> reclassify_as_equilibrium,\n\
           when attention < 5% -> reclassify_as_nonequilibrium,\n\
           when cpu_power >= 400W -> subvert,\n\
           when attention > 90% -> act(LedIndicador, \"verde\") }";
    let (p, d) = parse(fonte);
    assert!(!d.has_errors(), "{d}");
    let rules = &p.reviews().next().unwrap().rules;
    assert_eq!(rules[0].actions, vec![Action::NotifyShutdown]);
    assert_eq!(rules[1].actions, vec![Action::ReclassifyAsEquilibrium]);
    assert_eq!(rules[2].actions, vec![Action::ReclassifyAsNonequilibrium]);
    assert_eq!(rules[3].actions, vec![Action::Subvert]);
    match &rules[4].actions[0] {
        Action::Act { actor, value, .. } => {
            assert_eq!(actor, "LedIndicador");
            assert_eq!(value.kind, ExprKind::Str("verde".into()));
        }
        outro => panic!("esperado act, encontrado {outro:?}"),
    }
}

#[test]
fn producao_action_list_multiplas_acoes_na_ordem() {
    let fonte = "\
nonequilibrium A { value: \"v\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
review A { when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50) }";
    let (p, d) = parse(fonte);
    assert!(!d.has_errors(), "{d}");
    let acoes = &p.reviews().next().unwrap().rules[0].actions;
    assert_eq!(acoes.len(), 2);
    assert_eq!(acoes[0], Action::Subvert);
    match &acoes[1] {
        Action::Act { actor, value, .. } => {
            assert_eq!(actor, "CpuPowerCap");
            assert_eq!(value.kind, ExprKind::Num(50.0));
        }
        outro => panic!("esperado act, encontrado {outro:?}"),
    }
}

#[test]
fn producao_action_desconhecida_rejeitada() {
    assert!(contem(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 90°C -> explodir }",
        "acao_desconhecida"
    ));
}

// ======================================================================
// Produção: main_block / statement (keep | act | every)
// ======================================================================
#[test]
fn producao_main_block_keep_act_every() {
    let fonte = "\
nonequilibrium T { value: \"dados\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 5s }\n\
main { keep(T),\n       act(LedIndicador, \"verde\"),\n       every 4s { keep(T) },\n       every 10s { act(LedIndicador, \"verde\") } }";
    let (p, d) = parse(fonte);
    assert!(!d.has_errors(), "{d}");
    let main = p.main.as_ref().unwrap();
    assert_eq!(main.statements.len(), 4);
}

#[test]
fn producao_main_statement_desconhecido_e_keep_inexistente() {
    assert!(contem("main { voar() }", "statement_desconhecido"));
    assert!(contem(
        "event X { value: \"v\", horizon: 3s }\nmain { every 1s { keep(Inexistente) } }",
        "keep_forma_inexistente"
    ));
    // keep de forma declarada não produz diagnóstico
    sem_erros("event X { value: \"v\", horizon: 3s }\nmain { every 1s { keep(X) } }");
}

// ======================================================================
// Produções auxiliares: expression / duration / time_unit / number
// ======================================================================
#[test]
fn producao_duration_unidades_de_tempo_e_decimais() {
    let casos = [
        ("3s", 3.0, TimeUnit::S),
        ("500ms", 0.5, TimeUnit::Ms),
        ("200us", 0.0002, TimeUnit::Us),
        ("100ns", 0.0000001, TimeUnit::Ns),
        ("2.5s", 2.5, TimeUnit::S),
    ];
    for (txt, seg, unit) in casos {
        let fonte = format!("event X {{ value: \"v\", horizon: {txt} }}");
        let (p, d) = parse(&fonte);
        assert!(!d.has_errors(), "{txt}: {d}");
        let h = &p.forms().next().unwrap().horizon;
        assert!((h.segundos() - seg).abs() < 1e-12, "{txt} -> {}", h.segundos());
        assert_eq!(h.unit, unit);
    }
    // duração inválida
    assert!(contem("event X { value: \"v\", horizon: 3 }", "duracao_invalida"));
    assert!(contem("event X { value: \"v\", horizon: 3 parsecs }", "duracao_invalida"));
}

#[test]
fn producao_number_inteiro_e_decimal() {
    // decimal em threshold
    let (p, d) = parse(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 85.5°C -> dissolve }",
    );
    assert!(!d.has_errors());
    assert_eq!(p.reviews().next().unwrap().rules[0].threshold.valor, 85.5);
    // decimal inválido no lexer (ponto sem dígito)
    assert!(contem(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 85. °C -> dissolve }",
        "lexema_invalido"
    ));
}

// ======================================================================
// FORMAL §2 — unidades léxicas e comentários
// ======================================================================
#[test]
fn lexico_comentarios_de_linha_e_bloco() {
    sem_erros(
        "// comentário de linha\nevent X { /* bloco\n multilinha */ value: \"v\", horizon: 3s }",
    );
    // comentário de bloco sem fechamento é erro
    assert!(contem("event X { /* sem fim", "comentario_nao_fechado"));
}

#[test]
fn lexico_strings_escapes_e_limite_de_256_bytes() {
    let (p, d) = parse("event X { value: \"aspas\\\" barra\\\\ quebra\\n tab\\t\", horizon: 3s }");
    assert!(!d.has_errors(), "{d}");
    match &p.forms().next().unwrap().value.kind {
        ExprKind::Str(s) => assert_eq!(s, "aspas\" barra\\ quebra\n tab\t"),
        outro => panic!("{outro:?}"),
    }
    // 257 bytes → string_muito_longa (FORMAL §2)
    let longa = "x".repeat(257);
    assert!(contem(&format!("event X {{ value: \"{longa}\", horizon: 3s }}"), "string_muito_longa"));
    // 256 bytes passa
    let ok = "x".repeat(256);
    sem_erros(&format!("event X {{ value: \"{ok}\", horizon: 3s }}"));
    // string não terminada
    assert!(contem("event X { value: \"sem fim }", "string_nao_terminada"));
}

#[test]
fn lexico_lexema_invalido_com_linha_e_coluna() {
    let (_, d) = parse("event X {\n  value: \"v\"\n  @\n  horizon: 3s }");
    let diag = d.items.iter().find(|i| i.code == "lexema_invalido").expect("lexema_invalido");
    assert_eq!(diag.span.line, 3);
    assert!(diag.span.col >= 2);
}

// ======================================================================
// Exemplos canônicos da FORMAL §5 — todos validam sem erros
// ======================================================================
#[test]
fn exemplos_canonicos_da_formal_validam() {
    sem_erros(
        "nonequilibrium PensarLivre {\n\
         \x20   value: \"consciencia_anteneoliberal_ativa\",\n\
         \x20   horizon: 60s,\n\
         \x20   source_path: \"attention\",\n\
         \x20   maintenance_deadline: 3s,\n\
         \x20   exchange_mode: \"cooperation\"\n\
         }\n\
         review PensarLivre { when attention < 30% -> reclassify_as_equilibrium }",
    );
    sem_erros(
        "nonequilibrium TradingEspeculativo {\n\
         \x20   value: \"lucro_arbitragem_alta_frequencia\",\n\
         \x20   horizon: 7s,\n\
         \x20   source_path: \"cpu_temp\",\n\
         \x20   maintenance_deadline: 2s,\n\
         \x20   exchange_mode: \"extraction\"\n\
         }\n\
         review TradingEspeculativo {\n\
         \x20   when cpu_temp > 85°C -> subvert,\n\
         \x20                           act(CpuPowerCap, 50)\n\
         }",
    );
    sem_erros(
        "event Piscada {\n    value: \"impulso_curto\",\n    horizon: 2s\n}\n\
         review Piscada { when cpu_temp > 90°C -> dissolve }",
    );
    sem_erros(
        "equilibrium Registro {\n    value: \"documento_persistente\",\n    horizon: 86400s,\n    cost_bytes: 4096\n}",
    );
    sem_erros(
        "nonequilibrium TarefaImportante {\n\
         \x20   value: \"dados_sensiveis\",\n    horizon: 30s,\n    source_path: \"cpu_power\",\n\
         \x20   maintenance_deadline: 5s,\n    exchange_mode: \"cooperation\"\n}\n\
         main {\n    every 4s { keep(TarefaImportante) },\n    every 10s { act(LedIndicador, \"verde\") }\n}",
    );
}

#[test]
fn nota_regra_sem_seta_ou_com_estrutura_quebrada() {
    assert!(contem(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 90°C dissolve }",
        "regra_mal_formada"
    ));
    assert!(contem(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 90°C -> act(SemValor) }",
        "regra_mal_formada"
    ));
}

#[test]
fn producao_statement_every_muito_profundo_e_rejeitado() {
    // 8 níveis aninhados são aceitos; o 9º gera `every_muito_profundo`
    let aninhado = |prof: usize| {
        let mut fonte = String::from(
            "nonequilibrium T { value: \"v\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 5s }\nmain { ",
        );
        for _ in 0..prof {
            fonte.push_str("every 1s { ");
        }
        fonte.push_str("keep(T)");
        for _ in 0..prof {
            fonte.push_str(" }");
        }
        fonte.push_str(" }");
        fonte
    };
    sem_erros(&aninhado(8));
    assert!(contem(&aninhado(9), "every_muito_profundo"));
}
