//! Matriz de rastreabilidade do parser — produções EBNF × testes.
//!
//! Cada teste referencia explicitamente a produção da FORMAL §3 que cobre
//! (ver `docs/reports/STAGE-2-TRACEABILITY-MATRIX.md`): 100% das produções e
//! ≥ 95% das notas semânticas com ≥ 1 teste (critério "Done" da Etapa 2,
//! AGENTS.md §2.2).

use vbl_lang::parse;
use vbl_lang::{Action, CmpOp, Conjugation, Conjugation as Conj, ExprKind, PhysicalUnit, TimeUnit};

fn errors(text: &str) -> Vec<String> {
    let (_, d) = parse(text);
    d.items.iter().map(|i| i.code.clone()).collect()
}

fn no_errors(text: &str) {
    let (_, d) = parse(text);
    assert!(
        !d.has_errors(),
        "esperado programa válido, diagnósticos:\n{d}"
    );
}

fn contains(text: &str, code: &str) -> bool {
    errors(text).iter().any(|c| c == code)
}

// ======================================================================
// Produção: program = { form_declaration | review_declaration } [ main_block ]
// ======================================================================
#[test]
fn program_production_forms_reviews_and_main() {
    no_errors(
        "event A { value: \"v\", horizon: 3s }\n\
         review A { when cpu_temp > 90°C -> dissolve }\n\
         main { every 1s { keep(A) } }",
    );
}

#[test]
fn program_production_free_order_and_optional_main() {
    // main ausente; review antes da forma também é aceito (a ligação é
    // resolvida no conjunto de declarações)
    no_errors(
        "review A { when cpu_temp > 90°C -> dissolve }\nevent A { value: \"v\", horizon: 3s }",
    );
    no_errors("event A { value: \"v\", horizon: 3s }");
}

#[test]
fn program_production_main_must_be_last() {
    assert!(contains(
        "main { }\nevent A { value: \"v\", horizon: 3s }",
        "main_deve_ser_ultimo"
    ));
}

#[test]
fn program_production_duplicate_main_rejected() {
    assert!(contains("main { }\nmain { }", "main_duplicado"));
}

#[test]
fn program_production_invalid_top_level() {
    assert!(contains("42", "topo_invalido"));
    assert!(contains(
        "foo X { value: \"v\", horizon: 3s }",
        "topo_invalido"
    ));
}

// ======================================================================
// Produção: form_declaration / conjugation_kw
// ======================================================================
#[test]
fn production_conjugation_kw_three_variants() {
    for (conj_txt, esperada) in [
        ("event", Conj::Event),
        ("equilibrium", Conj::Equilibrium),
        ("nonequilibrium", Conj::Nonequilibrium),
    ] {
        let source = match esperada {
            Conj::Nonequilibrium => {
                format!("{conj_txt} X {{ value: \"v\", horizon: 3s, maintenance_deadline: 2s }}")
            }
            _ => format!("{conj_txt} X {{ value: \"v\", horizon: 3s }}"),
        };
        let (p, d) = parse(&source);
        assert!(!d.has_errors(), "{conj_txt}: {d}");
        let f = p.forms().next().unwrap();
        assert_eq!(f.conjugation, esperada);
    }
}

#[test]
fn production_form_declaration_structure() {
    // sem identificador / sem '{' / sem '}'
    assert!(contains(
        "event { value: \"v\", horizon: 3s }",
        "estrutura_forma"
    ));
    assert!(contains("event X value: \"v\" }", "estrutura_forma"));
    assert!(contains(
        "event X { value: \"v\", horizon: 3s",
        "bloco_nao_fechado"
    ));
}

#[test]
fn production_form_declaration_duplicate() {
    assert!(contains(
        "event X { value: \"v\", horizon: 3s }\nevent X { value: \"w\", horizon: 4s }",
        "forma_duplicada"
    ));
}

// ======================================================================
// Produção: form_body + cláusulas de Lei 1 (value/horizon)
// ======================================================================
#[test]
fn production_form_body_value_and_horizon_required_in_this_order() {
    assert!(contains("event X { horizon: 3s }", "value_obrigatorio"));
    assert!(contains("event X { value: \"v\" }", "horizon_obrigatorio"));
    assert!(contains(
        "event X { horizon: 3s, value: \"v\" }",
        "ordem_value_horizon"
    ));
}

#[test]
fn note_law1_value_is_opaque_to_runtime() {
    // expression aceita string, número e identificador (nota FORMAL §3)
    let (p, d) = parse("event X { value: \"s\", horizon: 3s }");
    assert!(!d.has_errors());
    assert_eq!(
        p.forms().next().unwrap().value.kind,
        ExprKind::Str("s".into())
    );

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
fn production_form_body_trailing_comma_rejected() {
    assert!(contains(
        "event X { value: \"v\", horizon: 3s, }",
        "virgula_final"
    ));
    assert!(contains("main { every 1s { keep(X) }, }", "virgula_final"));
}

// ======================================================================
// Produção: optional_attribute (6 variantes)
// ======================================================================
#[test]
fn production_optional_attribute_source_path() {
    let (p, d) = parse(
        "nonequilibrium X { value: \"v\", horizon: 3s, source_path: \"cpu_temp\", maintenance_deadline: 2s }",
    );
    assert!(!d.has_errors(), "{d}");
    assert_eq!(
        p.forms().next().unwrap().attrs.source_path.as_deref(),
        Some("cpu_temp")
    );
}

#[test]
fn note_source_path_exclusively_symbolic() {
    // caminhos de SO não são source_path válidos (FORMAL §3, nota)
    assert!(contains(
        "nonequilibrium X { value: \"v\", horizon: 3s, source_path: \"/sys/class/thermal/temp\", maintenance_deadline: 2s }",
        "source_path_nao_simbolico"
    ));
    assert!(contains(
        "nonequilibrium X { value: \"v\", horizon: 3s, source_path: \"./local\", maintenance_deadline: 2s }",
        "source_path_nao_simbolico"
    ));
}

#[test]
fn production_optional_attribute_maintenance_deadline() {
    let (p, d) =
        parse("nonequilibrium X { value: \"v\", horizon: 3s, maintenance_deadline: 2.5s }");
    assert!(!d.has_errors(), "{d}");
    let f = p.forms().next().unwrap();
    assert_eq!(f.attrs.maintenance_deadline.unwrap().seconds(), 2.5);
}

#[test]
fn note_maintenance_deadline_required_in_nonequilibrium() {
    assert!(contains(
        "nonequilibrium X { value: \"v\", horizon: 3s }",
        "maintenance_deadline_ausente"
    ));
    // e proibido nas demais conjugações
    assert!(contains(
        "event X { value: \"v\", horizon: 3s, maintenance_deadline: 2s }",
        "atributo_nao_aplicavel"
    ));
}

#[test]
fn production_optional_attribute_exchange_mode() {
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
fn note_exchange_mode_only_nonequilibrium() {
    assert!(contains(
        "equilibrium X { value: \"v\", horizon: 3s, exchange_mode: \"cooperation\" }",
        "atributo_nao_aplicavel"
    ));
}

#[test]
fn production_optional_attribute_cost_bytes() {
    let (p, d) = parse("equilibrium X { value: \"v\", horizon: 3s, cost_bytes: 4096 }");
    assert!(!d.has_errors(), "{d}");
    assert_eq!(p.forms().next().unwrap().attrs.cost_bytes, Some(4096));
    // decimal é rejeitado (integer na EBNF)
    assert!(contains(
        "equilibrium X { value: \"v\", horizon: 3s, cost_bytes: 4.5 }",
        "cost_bytes_inteiro"
    ));
}

#[test]
fn note_cost_bytes_only_equilibrium() {
    assert!(contains(
        "event X { value: \"v\", horizon: 3s, cost_bytes: 16 }",
        "atributo_nao_aplicavel"
    ));
}

#[test]
fn production_optional_attribute_currency() {
    let (p, d) = parse("event X { value: \"v\", horizon: 3s, currency: \"CpuCycles\" }");
    assert!(!d.has_errors(), "{d}");
    assert_eq!(
        p.forms().next().unwrap().attrs.currency.as_deref(),
        Some("CpuCycles")
    );
}

#[test]
fn note_currency_inherits_conjugation_default() {
    // padrões canônicos: CpuCycles/DiskBytes/PowerWatts (FORMAL §3)
    assert_eq!(Conjugation::Event.default_currency(), "CpuCycles");
    assert_eq!(Conjugation::Equilibrium.default_currency(), "DiskBytes");
    assert_eq!(Conjugation::Nonequilibrium.default_currency(), "PowerWatts");
}

#[test]
fn production_optional_attribute_classification() {
    let (p, d) = parse("event X { value: \"v\", horizon: 3s, classification: \"Transiente\" }");
    assert!(!d.has_errors(), "{d}");
    assert_eq!(
        p.forms().next().unwrap().attrs.classification.as_deref(),
        Some("Transiente")
    );
}

#[test]
fn production_optional_attribute_unknown_and_duplicate() {
    assert!(contains(
        "event X { value: \"v\", horizon: 3s, foo: 1 }",
        "atributo_desconhecido"
    ));
    assert!(contains(
        "event X { value: \"v\", horizon: 3s, horizon: 4s }",
        "atributo_duplicado"
    ));
}

// ======================================================================
// Produção: review_declaration / review_rule / sensor_ref / comparison_op
// ======================================================================
#[test]
fn production_review_declaration_with_rules() {
    let (p, d) = parse(
        "event A { value: \"v\", horizon: 3s }\n\
         review A { when cpu_temp > 90°C -> dissolve,\n\
                    when cpu_temp < 10°C -> notify_shutdown }",
    );
    assert!(!d.has_errors(), "{d}");
    assert_eq!(p.reviews().next().unwrap().rules.len(), 2);
}

#[test]
fn note_review_orphan_and_duplicate_are_compile_errors() {
    assert!(contains(
        "event X { value: \"v\", horizon: 3s }\nreview Y { when cpu_temp > 90°C -> dissolve }",
        "review_orfa"
    ));
    assert!(contains(
        "event X { value: \"v\", horizon: 3s }\n\
         review X { when cpu_temp > 90°C -> dissolve }\n\
         review X { when cpu_temp < 10°C -> dissolve }",
        "review_duplicada"
    ));
}

#[test]
fn production_sensor_ref_identifier_or_string() {
    let (p, d) = parse(
        "event A { value: \"v\", horizon: 3s }\n\
         review A { when \"cpu_temp\" > 90°C -> dissolve }",
    );
    assert!(!d.has_errors(), "{d}");
    assert_eq!(p.reviews().next().unwrap().rules[0].sensor.name, "cpu_temp");
    // sem sensor → erro
    assert!(contains(
        "event A { value: \"v\", horizon: 3s }\nreview A { when > 90°C -> dissolve }",
        "regra_mal_formada"
    ));
}

#[test]
fn production_comparison_op_six_operators() {
    for (op, expected) in [
        ("<", CmpOp::Lt),
        (">", CmpOp::Gt),
        ("<=", CmpOp::Le),
        (">=", CmpOp::Ge),
        ("==", CmpOp::Eq),
        ("!=", CmpOp::Ne),
    ] {
        let source = format!(
            "event A {{ value: \"v\", horizon: 3s }}\nreview A {{ when cpu_temp {op} 90°C -> dissolve }}"
        );
        let (p, d) = parse(&source);
        assert!(!d.has_errors(), "op {op}: {d}");
        assert_eq!(p.reviews().next().unwrap().rules[0].op, expected);
    }
    // operadores inválidos
    assert!(contains(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp = 90°C -> dissolve }",
        "lexema_invalido"
    ));
    assert!(contains(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp ~ 90°C -> dissolve }",
        "operador_invalido"
    ));
}

// ======================================================================
// Produção: threshold / percentage / physical_quantity
// ======================================================================
#[test]
fn production_threshold_number_percentage_and_physical_quantity() {
    let casos = [
        ("when cpu_temp > 90 -> None", 90.0, None),
        (
            "when attention < 30% -> Some(PhysicalUnit::Percent)",
            30.0,
            Some(PhysicalUnit::Percent),
        ),
        (
            "when cpu_temp > 85°C -> Some(PhysicalUnit::DegC)",
            85.0,
            Some(PhysicalUnit::DegC),
        ),
        (
            "when cpu_power >= 150W -> Some(PhysicalUnit::W)",
            150.0,
            Some(PhysicalUnit::W),
        ),
    ];
    for (rule, value, unit) in casos {
        let rule = rule
            .replace("-> None", "-> dissolve")
            .replace("-> Some(PhysicalUnit::Percent)", "-> dissolve")
            .replace("-> Some(PhysicalUnit::DegC)", "-> dissolve")
            .replace("-> Some(PhysicalUnit::W)", "-> dissolve");
        let source = format!("event A {{ value: \"v\", horizon: 3s }}\nreview A {{ {rule} }}");
        let (p, d) = parse(&source);
        assert!(!d.has_errors(), "regra `{rule}`: {d}");
        let t = &p.reviews().next().unwrap().rules[0].threshold;
        assert_eq!(t.value, value);
        assert_eq!(t.unit, unit);
    }
}

#[test]
fn note_threshold_unit_is_captured_for_registry_validation() {
    // A unidade é preservada na AST para validação contra a grandeza do
    // sensor no registro do FXP (FORMAL §3, nota; loader valida em runtime).
    let (p, d) = parse(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 90°C -> dissolve }",
    );
    assert!(!d.has_errors());
    assert_eq!(
        p.reviews().next().unwrap().rules[0].threshold.unit,
        Some(PhysicalUnit::DegC)
    );
    // threshold não-numérico é rejeitado
    assert!(contains(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > alto -> dissolve }",
        "regra_mal_formada"
    ));
}

// ======================================================================
// Produção: action_list / action (6 variantes)
// ======================================================================
#[test]
fn production_action_six_variants_in_declared_order() {
    let source = "\
event A { value: \"v\", horizon: 30s }\n\
review A { when cpu_temp > 10°C -> dissolve }\n\
review X { when cpu_temp > 10°C -> dissolve }";
    let _ = source; // (cobertura simples abaixo)
    let source = "\
nonequilibrium A { value: \"v\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
review A { when cpu_temp > 10°C -> notify_shutdown,\n\
           when cpu_temp < 5°C -> reclassify_as_equilibrium,\n\
           when attention < 5% -> reclassify_as_nonequilibrium,\n\
           when cpu_power >= 400W -> subvert,\n\
           when attention > 90% -> act(StatusLed, \"green\") }";
    let (p, d) = parse(source);
    assert!(!d.has_errors(), "{d}");
    let rules = &p.reviews().next().unwrap().rules;
    assert_eq!(rules[0].actions, vec![Action::NotifyShutdown]);
    assert_eq!(rules[1].actions, vec![Action::ReclassifyAsEquilibrium]);
    assert_eq!(rules[2].actions, vec![Action::ReclassifyAsNonequilibrium]);
    assert_eq!(rules[3].actions, vec![Action::Subvert]);
    match &rules[4].actions[0] {
        Action::Act { actor, value, .. } => {
            assert_eq!(actor, "StatusLed");
            assert_eq!(value.kind, ExprKind::Str("green".into()));
        }
        other => panic!("esperado act, encontrado {other:?}"),
    }
}

#[test]
fn production_action_list_multiple_actions_in_order() {
    let source = "\
nonequilibrium A { value: \"v\", horizon: 30s, source_path: \"cpu_temp\", maintenance_deadline: 10s }\n\
review A { when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50) }";
    let (p, d) = parse(source);
    assert!(!d.has_errors(), "{d}");
    let actions = &p.reviews().next().unwrap().rules[0].actions;
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0], Action::Subvert);
    match &actions[1] {
        Action::Act { actor, value, .. } => {
            assert_eq!(actor, "CpuPowerCap");
            assert_eq!(value.kind, ExprKind::Num(50.0));
        }
        other => panic!("esperado act, encontrado {other:?}"),
    }
}

#[test]
fn production_action_unknown_rejected() {
    assert!(contains(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 90°C -> explodir }",
        "acao_desconhecida"
    ));
}

// ======================================================================
// Produção: main_block / statement (keep | act | every)
// ======================================================================
#[test]
fn production_main_block_keep_act_every() {
    let source = "\
nonequilibrium T { value: \"dados\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 5s }\n\
main { keep(T),\n       act(StatusLed, \"green\"),\n       every 4s { keep(T) },\n       every 10s { act(StatusLed, \"green\") } }";
    let (p, d) = parse(source);
    assert!(!d.has_errors(), "{d}");
    let main = p.main.as_ref().unwrap();
    assert_eq!(main.statements.len(), 4);
}

#[test]
fn production_main_statement_unknown_and_missing_keep() {
    assert!(contains("main { voar() }", "statement_desconhecido"));
    assert!(contains(
        "event X { value: \"v\", horizon: 3s }\nmain { every 1s { keep(Inexistente) } }",
        "keep_forma_inexistente"
    ));
    // keep de forma declarada não produz diagnóstico
    no_errors("event X { value: \"v\", horizon: 3s }\nmain { every 1s { keep(X) } }");
}

// ======================================================================
// Produções auxiliares: expression / duration / time_unit / number
// ======================================================================
#[test]
fn production_duration_time_units_and_decimals() {
    let casos = [
        ("3s", 3.0, TimeUnit::S),
        ("500ms", 0.5, TimeUnit::Ms),
        ("200us", 0.0002, TimeUnit::Us),
        ("100ns", 0.0000001, TimeUnit::Ns),
        ("2.5s", 2.5, TimeUnit::S),
    ];
    for (txt, secs, unit) in casos {
        let source = format!("event X {{ value: \"v\", horizon: {txt} }}");
        let (p, d) = parse(&source);
        assert!(!d.has_errors(), "{txt}: {d}");
        let h = &p.forms().next().unwrap().horizon;
        assert!(
            (h.seconds() - secs).abs() < 1e-12,
            "{txt} -> {}",
            h.seconds()
        );
        assert_eq!(h.unit, unit);
    }
    // duração inválida
    assert!(contains(
        "event X { value: \"v\", horizon: 3 }",
        "duracao_invalida"
    ));
    assert!(contains(
        "event X { value: \"v\", horizon: 3 parsecs }",
        "duracao_invalida"
    ));
}

#[test]
fn production_number_integer_and_decimal() {
    // decimal em threshold
    let (p, d) = parse(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 85.5°C -> dissolve }",
    );
    assert!(!d.has_errors());
    assert_eq!(p.reviews().next().unwrap().rules[0].threshold.value, 85.5);
    // decimal inválido no lexer (ponto sem dígito)
    assert!(contains(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 85. °C -> dissolve }",
        "lexema_invalido"
    ));
}

// ======================================================================
// FORMAL §2 — unidades léxicas e comentários
// ======================================================================
#[test]
fn lexical_line_and_block_comments() {
    no_errors(
        "// comentário de linha\nevent X { /* bloco\n multilinha */ value: \"v\", horizon: 3s }",
    );
    // comentário de bloco sem fechamento é erro
    assert!(contains("event X { /* sem fim", "comentario_nao_fechado"));
}

#[test]
fn lexical_strings_escapes_and_256_byte_limit() {
    let (p, d) = parse("event X { value: \"aspas\\\" barra\\\\ quebra\\n tab\\t\", horizon: 3s }");
    assert!(!d.has_errors(), "{d}");
    match &p.forms().next().unwrap().value.kind {
        ExprKind::Str(s) => assert_eq!(s, "aspas\" barra\\ quebra\n tab\t"),
        other => panic!("{other:?}"),
    }
    // 257 bytes → string_muito_longa (FORMAL §2)
    let long = "x".repeat(257);
    assert!(contains(
        &format!("event X {{ value: \"{long}\", horizon: 3s }}"),
        "string_muito_longa"
    ));
    // 256 bytes passa
    let ok = "x".repeat(256);
    no_errors(&format!("event X {{ value: \"{ok}\", horizon: 3s }}"));
    // string não terminada
    assert!(contains(
        "event X { value: \"sem fim }",
        "string_nao_terminada"
    ));
}

#[test]
fn lexical_invalid_lexeme_with_line_and_column() {
    let (_, d) = parse("event X {\n  value: \"v\"\n  @\n  horizon: 3s }");
    let diag = d
        .items
        .iter()
        .find(|i| i.code == "lexema_invalido")
        .expect("lexema_invalido");
    assert_eq!(diag.span.line, 3);
    assert!(diag.span.col >= 2);
}

// ======================================================================
// Exemplos canônicos da FORMAL §5 — todos validam sem erros
// ======================================================================
#[test]
fn canonical_examples_from_formal_validate() {
    no_errors(
        "nonequilibrium FreeThinking {\n\
         \x20   value: \"consciencia_antineoliberal_ativa\",\n\
         \x20   horizon: 60s,\n\
         \x20   source_path: \"attention\",\n\
         \x20   maintenance_deadline: 3s,\n\
         \x20   exchange_mode: \"cooperation\"\n\
         }\n\
         review FreeThinking { when attention < 30% -> reclassify_as_equilibrium }",
    );
    no_errors(
        "nonequilibrium SpeculativeTrading {\n\
         \x20   value: \"lucro_arbitragem_alta_frequencia\",\n\
         \x20   horizon: 7s,\n\
         \x20   source_path: \"cpu_temp\",\n\
         \x20   maintenance_deadline: 2s,\n\
         \x20   exchange_mode: \"extraction\"\n\
         }\n\
         review SpeculativeTrading {\n\
         \x20   when cpu_temp > 85°C -> subvert,\n\
         \x20                           act(CpuPowerCap, 50)\n\
         }",
    );
    no_errors(
        "event Piscada {\n    value: \"impulso_curto\",\n    horizon: 2s\n}\n\
         review Piscada { when cpu_temp > 90°C -> dissolve }",
    );
    no_errors(
        "equilibrium Registro {\n    value: \"documento_persistente\",\n    horizon: 86400s,\n    cost_bytes: 4096\n}",
    );
    no_errors(
        "nonequilibrium ImportantTask {\n\
         \x20   value: \"dados_sensiveis\",\n    horizon: 30s,\n    source_path: \"cpu_power\",\n\
         \x20   maintenance_deadline: 5s,\n    exchange_mode: \"cooperation\"\n}\n\
         main {\n    every 4s { keep(ImportantTask) },\n    every 10s { act(StatusLed, \"green\") }\n}",
    );
}

#[test]
fn note_rule_without_arrow_or_broken_structure() {
    assert!(contains(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 90°C dissolve }",
        "regra_mal_formada"
    ));
    assert!(contains(
        "event A { value: \"v\", horizon: 3s }\nreview A { when cpu_temp > 90°C -> act(SemValor) }",
        "regra_mal_formada"
    ));
}

#[test]
fn production_statement_every_too_deep_and_rejected() {
    // 8 níveis aninhados são aceitos; o 9º gera `every_muito_profundo`
    let nested = |depth: usize| {
        let mut source = String::from(
            "nonequilibrium T { value: \"v\", horizon: 30s, source_path: \"cpu_power\", maintenance_deadline: 5s }\nmain { ",
        );
        for _ in 0..depth {
            source.push_str("every 1s { ");
        }
        source.push_str("keep(T)");
        for _ in 0..depth {
            source.push_str(" }");
        }
        source.push_str(" }");
        source
    };
    no_errors(&nested(8));
    assert!(contains(&nested(9), "every_muito_profundo"));
}
