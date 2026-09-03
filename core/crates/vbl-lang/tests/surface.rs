//! Superfície do vbl-lang: unidades léxicas, diagnósticos e acessores do AST.
//!
//! Complementa `productions_matrix.rs` (foco no parser) exercendo os
//! caminhos que o parser válido não alcança: braços de erro do lexer,
//! `Token::text()` para exibição em mensagens, `Display` dos diagnósticos
//! e os acessores canônicos do AST (FORMAL §2/§3).

use vbl_lang::lexer::tokenize;
use vbl_lang::token::{Token, TokenKind};
use vbl_lang::{
    CmpOp, Conjugation, Diagnostic, Diagnostics, ExprKind, Expression, PhysicalUnit, Span, TimeUnit,
};

fn diags_de(texto: &str) -> Vec<String> {
    let (_, d) = tokenize(texto);
    d.items.iter().map(|i| i.code.clone()).collect()
}

// ── token.rs — texto de exibição de cada categoria (mensagens de erro) ────
#[test]
fn token_text_cobre_todas_as_categorias() {
    let casos: Vec<(TokenKind, &str)> = vec![
        (TokenKind::Ident("chuva".into()), "chuva"),
        (TokenKind::Int(42), "42"),
        (TokenKind::Decimal(3.5), "3.5"),
        (TokenKind::Str("olá".into()), "\"olá\""),
        (TokenKind::DegC, "°C"),
        (TokenKind::Percent, "%"),
        (TokenKind::Colon, ":"),
        (TokenKind::Comma, ","),
        (TokenKind::Semi, ";"),
        (TokenKind::LBrace, "{"),
        (TokenKind::RBrace, "}"),
        (TokenKind::LParen, "("),
        (TokenKind::RParen, ")"),
        (TokenKind::Arrow, "->"),
        (TokenKind::Lt, "<"),
        (TokenKind::Gt, ">"),
        (TokenKind::Le, "<="),
        (TokenKind::Ge, ">="),
        (TokenKind::EqEq, "=="),
        (TokenKind::NotEq, "!="),
    ];
    for (kind, texto) in casos {
        let t = Token {
            kind,
            span: Span::new(1, 1),
        };
        assert_eq!(t.text(), texto);
    }
}

// ── diag.rs — gravidade, Display, ordenação e consultas ───────────────────
#[test]
fn span_display_e_default() {
    assert_eq!(Span::default(), Span { line: 0, col: 0 });
    assert_eq!(Span::new(3, 7).to_string(), "3:7");
}

#[test]
fn diagnostic_severidade_e_display() {
    let erro = Diagnostic::error("erro_teste", Span::new(2, 10), "falhou");
    assert!(erro.is_error());
    assert_eq!(erro.to_string(), "2:10 [erro] erro_teste: falhou");

    let aviso = Diagnostic::warning("aviso_teste", Span::new(1, 1), "atenção");
    assert!(!aviso.is_error());
    assert_eq!(aviso.to_string(), "1:1 [aviso] aviso_teste: atenção");
}

#[test]
fn diagnostics_consultas_ordenacao_e_display() {
    let mut a = Diagnostics::new();
    a.error("b_code", Span::new(2, 1), "depois");
    a.warning("a_code", Span::new(1, 5), "antes");
    assert!(a.has_errors());
    assert_eq!(a.errors().count(), 1);

    let mut b = Diagnostics::new();
    b.error("c_code", Span::new(3, 1), "de outro");
    a.extend(b);

    a.sort();
    let linhas: Vec<u32> = a.items.iter().map(|d| d.span.line).collect();
    assert_eq!(linhas, vec![1, 2, 3]);

    assert!(a.contains("c_code"));
    assert!(!a.contains("x_code"));
    let codigos = a.codes();
    assert_eq!(codigos.len(), 3);

    let texto = a.to_string();
    assert_eq!(texto.lines().count(), 3);
    assert!(texto.contains("1:5"));

    let vazio = Diagnostics::default();
    assert!(!vazio.has_errors());
    assert_eq!(vazio.to_string(), ""); // Display de coleção vazia
}

// ── lexer.rs — braços de erro (FORMAL §2: léxico estrito) ─────────────────
#[test]
fn lexemas_invalidos_tem_codigo_estavel() {
    assert!(diags_de("=").iter().any(|c| c == "lexema_invalido"));
    assert!(diags_de("!").iter().any(|c| c == "lexema_invalido"));
    assert!(diags_de("a - b").iter().any(|c| c == "lexema_invalido"));
    assert!(diags_de("@").iter().any(|c| c == "lexema_invalido"));
    assert!(diags_de("°F").iter().any(|c| c == "lexema_invalido"));
    assert!(diags_de("°").iter().any(|c| c == "lexema_invalido"));
    // os válidos, para contraste
    let (tokens, d) = tokenize("a -> b <= c >= d == e != f < g > h");
    assert!(d.items.is_empty());
    assert_eq!(tokens.len(), 15);
}

#[test]
fn string_nao_terminada_nas_tres_formas() {
    // EOF sem fechar
    assert!(diags_de("\"aberta")
        .iter()
        .any(|c| c == "string_nao_terminada"));
    // quebra de linha dentro da string
    assert!(diags_de("\"duas\nlinhas\"")
        .iter()
        .any(|c| c == "string_nao_terminada"));
    // string termina em escape solitário
    assert!(diags_de("\"escape\\")
        .iter()
        .any(|c| c == "string_nao_terminada"));
}

#[test]
fn escapes_de_string_canonicos_e_extras() {
    let (tokens, d) = tokenize("\"a\\\"b\\\\c\\n\\t\\x\"");
    assert!(d.items.is_empty()); // \x não canônico preserva o caractere
    match &tokens[0].kind {
        TokenKind::Str(s) => assert_eq!(s, "a\"b\\c\n\tx"),
        other => panic!("esperado Str, veio {other:?}"),
    }
}

#[test]
fn string_muito_longa_rejeitada() {
    let longa = format!("\"{}\"", "x".repeat(257)); // limite: 256 bytes
    assert!(diags_de(&longa).iter().any(|c| c == "string_muito_longa"));
    let no_limite = format!("\"{}\"", "x".repeat(256));
    assert!(!diags_de(&no_limite)
        .iter()
        .any(|c| c == "string_muito_longa"));
}

#[test]
fn comentario_de_bloco_nao_fechado() {
    assert!(diags_de("/* sem fim")
        .iter()
        .any(|c| c == "comentario_nao_fechado"));
    // fechado corretamente não diagnosticа
    let (_, d) = tokenize("/* ok */ event A { value: 1, horizon: 1s }");
    assert!(!d.has_errors());
}

// ── ast.rs — acessores canônicos (FORMAL §2/§3) ───────────────────────────
#[test]
fn conjugacoes_nome_e_moeda_padrao() {
    assert_eq!(Conjugation::Event.name(), "event");
    assert_eq!(Conjugation::Event.default_currency(), "CpuCycles");
    assert_eq!(Conjugation::Equilibrium.name(), "equilibrium");
    assert_eq!(Conjugation::Equilibrium.default_currency(), "DiskBytes");
    assert_eq!(Conjugation::Nonequilibrium.name(), "nonequilibrium");
    assert_eq!(Conjugation::Nonequilibrium.default_currency(), "PowerWatts");
}

#[test]
fn unidades_de_tempo_fator_e_sufixo() {
    let casos = [
        (TimeUnit::S, 1.0, "s"),
        (TimeUnit::Ms, 1e-3, "ms"),
        (TimeUnit::Us, 1e-6, "us"),
        (TimeUnit::Ns, 1e-9, "ns"),
    ];
    for (unidade, fator, sufixo) in casos {
        assert_eq!(unidade.factor(), fator);
        assert_eq!(unidade.suffix(), sufixo);
    }
    let dur = vbl_lang::Duration {
        value: 250.0,
        unit: TimeUnit::Ms,
        span: Span::default(),
    };
    assert_eq!(dur.seconds(), 0.25);
}

#[test]
fn unidades_fisicas_simbolo_e_grandeza() {
    let casos = [
        (PhysicalUnit::W, "W", "power"),
        (PhysicalUnit::DegC, "°C", "temperature"),
        (PhysicalUnit::Percent, "%", "attention"),
    ];
    for (unidade, simbolo, grandeza) in casos {
        assert_eq!(unidade.symbol(), simbolo);
        assert_eq!(unidade.quantity(), grandeza);
    }
}

#[test]
fn cmpop_simbolo_e_avaliacao() {
    // (op, símbolo, sensor, limiar, esperado) — limiar 5.0 fixo
    let casos = [
        (CmpOp::Lt, "<", 4.0, true),
        (CmpOp::Lt, "<", 6.0, false),
        (CmpOp::Gt, ">", 6.0, true),
        (CmpOp::Gt, ">", 4.0, false),
        (CmpOp::Le, "<=", 5.0, true),
        (CmpOp::Le, "<=", 5.1, false),
        (CmpOp::Ge, ">=", 5.0, true),
        (CmpOp::Ge, ">=", 4.9, false),
        (CmpOp::Eq, "==", 5.0, true),
        (CmpOp::Eq, "==", 4.9, false),
        (CmpOp::Ne, "!=", 4.0, true),
        (CmpOp::Ne, "!=", 5.0, false),
    ];
    for (op, simbolo, sensor, esperado) in casos {
        assert_eq!(op.symbol(), simbolo);
        assert_eq!(op.evaluate(sensor, 5.0), esperado, "{op:?}({sensor})");
    }
}

#[test]
fn expr_kind_construtores() {
    assert_eq!(
        Expression::str("poesia", Span::new(1, 2)).kind,
        ExprKind::Str("poesia".into())
    );
    assert_eq!(
        Expression::num(3.5, Span::new(1, 3)).kind,
        ExprKind::Num(3.5)
    );
    assert_eq!(
        Expression::ident("chuva", Span::new(1, 4)).kind,
        ExprKind::Ident("chuva".into())
    );
}
