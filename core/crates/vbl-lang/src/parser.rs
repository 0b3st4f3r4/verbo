//! Parser de descida recursiva sobre a EBNF da FORMAL §3.
//!
//! Produz a [`ast::Program`] e TODOS os diagnósticos encontrados (sem parar
//! no primeiro — recuperação por vírgula/fechamento de bloco). As cláusulas
//! de erro da FORMAL §3 são erros de compilação com códigos canônicos que
//! reancoram a matriz da Etapa 1:
//!
//! - `value`/`horizon` obrigatórios e nesta ordem (Lei 1);
//! - `maintenance_deadline`/`exchange_mode` só em `nonequilibrium`
//!   (obrigatório o primeiro nela); `cost_bytes` só em `equilibrium`;
//! - `review` órfã/duplicada; `keep` de forma inexistente; forma duplicada;
//! - `source_path` exclusivamente simbólico; vírgula final rejeitada;
//! - `main` único e por último.

use crate::ast::*;
use crate::diag::{Diagnostics, Span};
use crate::lexer::tokenize;
use crate::token::{kw, Token, TokenKind};

/// Limite de aninhamento de `every` (defesa contra recursão profunda).
const MAX_EVERY_DEPTH: usize = 8;

/// Faz o parse do fonte `.vl`: devolve programa + diagnósticos.
/// O programa só é considerado válido se `diags.has_errors() == false`.
pub fn parse(source: &str) -> (Program, Diagnostics) {
    let (tokens, mut diags) = tokenize(source);
    let mut p = Parser {
        toks: tokens,
        pos: 0,
        diags: Diagnostics::new(),
    };
    let program = p.program();
    p.cross_clauses(&program);
    diags.extend(p.diags);
    diags.sort();
    (program, diags)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Diagnostics,
}

impl Parser {
    // ------------------------------------------------------------------
    // Utilidades
    // ------------------------------------------------------------------
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&TokenKind> {
        self.toks.get(self.pos).map(|t| &t.kind)
    }

    fn advance(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn here(&self) -> Span {
        self.toks.last().map(|t| t.span).unwrap_or_default()
    }

    /// Consome o token esperado ou registra `code`.
    fn expect_punct(&mut self, kind: &TokenKind, code: &str, msg: &str) -> Option<Token> {
        match self.peek() {
            Some(t) if &t.kind == kind => self.advance(),
            Some(t) => {
                let found = t.text();
                let span = t.span;
                self.diags
                    .error(code, span, format!("{msg} (encontrado {found})"));
                None
            }
            None => {
                let span = self.here();
                self.diags
                    .error(code, span, format!("fim inesperado: {msg}"));
                None
            }
        }
    }

    /// Consome `,` se presente; registra vírgula final antes de `}`.
    fn consume_comma(&mut self, context: &str) {
        if self.is_punct(TokenKind::Comma) {
            self.advance();
            if self.is_punct(TokenKind::RBrace) {
                let span = self.peek().unwrap().span;
                self.diags.error(
                    "virgula_final",
                    span,
                    format!("vírgula final antes de '}}' em {context} (EBNF não permite)"),
                );
            }
        }
    }

    fn is_punct(&self, kind: TokenKind) -> bool {
        matches!(self.peek_kind(), Some(k) if *k == kind)
    }

    /// Descarta tokens até vírgula ou fechamento de bloco (recuperação).
    fn consume_until_comma_or_close(&mut self) {
        let mut depth = 0usize;
        while let Some(t) = self.peek() {
            match t.kind {
                TokenKind::LBrace | TokenKind::LParen => depth += 1,
                TokenKind::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                TokenKind::RParen => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                TokenKind::Comma if depth == 0 => return,
                _ => {}
            }
            self.advance();
        }
    }

    // ------------------------------------------------------------------
    // program = { form_declaration | review_declaration } [ main_block ]
    // ------------------------------------------------------------------
    fn program(&mut self) -> Program {
        let mut program = Program::default();
        let mut main_visto: Option<Span> = None;
        while let Some(tok) = self.peek().cloned() {
            let TokenKind::Ident(name) = &tok.kind else {
                let found = tok.text();
                let span = tok.span;
                self.diags.error(
                    "topo_invalido",
                    span,
                    format!("esperada declaração (forma/review/main), encontrado {found}"),
                );
                self.advance();
                continue;
            };
            match name.as_str() {
                "event" | "equilibrium" | "nonequilibrium" => {
                    if main_visto.is_some() {
                        self.diags.error(
                            "main_deve_ser_ultimo",
                            tok.span,
                            "declaração após o bloco `main` (EBNF: main é o último item)",
                        );
                    }
                    if let Some(f) = self.form(tok.span) {
                        program.decls.push(Declaration::Form(Box::new(f)));
                    }
                }
                "review" => {
                    if main_visto.is_some() {
                        self.diags.error(
                            "main_deve_ser_ultimo",
                            tok.span,
                            "declaração após o bloco `main` (EBNF: main é o último item)",
                        );
                    }
                    if let Some(r) = self.review(tok.span) {
                        program.decls.push(Declaration::Review(r));
                    }
                }
                "main" => {
                    if let Some(previous) = main_visto {
                        self.diags.error(
                            "main_duplicado",
                            tok.span,
                            format!("segundo bloco `main` (o primeiro está em {})", previous),
                        );
                        self.advance();
                        self.consume_until_comma_or_close();
                        continue;
                    }
                    main_visto = Some(tok.span);
                    if let Some(m) = self.main_block(tok.span) {
                        program.main = Some(m);
                    }
                }
                other => {
                    self.diags.error(
                        "topo_invalido",
                        tok.span,
                        format!(
                            "declaração desconhecida '{other}' (esperado forma, review ou main)"
                        ),
                    );
                    self.advance();
                }
            }
        }
        program
    }

    // ------------------------------------------------------------------
    // form_declaration = conjugation_kw identifier '{' form_body '}'
    // ------------------------------------------------------------------
    fn form(&mut self, kw_span: Span) -> Option<FormDecl> {
        let conjugation = match self.peek_kind() {
            Some(TokenKind::Ident(s)) => match s.as_str() {
                "event" => {
                    self.advance();
                    Conjugation::Event
                }
                "equilibrium" => {
                    self.advance();
                    Conjugation::Equilibrium
                }
                "nonequilibrium" => {
                    self.advance();
                    Conjugation::Nonequilibrium
                }
                _ => unreachable!("chamado apenas com conjugação"),
            },
            _ => unreachable!(),
        };
        let name = match self.advance() {
            Some(Token {
                kind: TokenKind::Ident(n),
                span,
            }) => (n, span),
            Some(t) => {
                let found = t.text();
                self.diags.error(
                    "estrutura_forma",
                    t.span,
                    format!("esperado identificador da forma (encontrado {found})"),
                );
                return None;
            }
            None => {
                self.diags.error(
                    "estrutura_forma",
                    kw_span,
                    "fim inesperado: esperado identificador da forma",
                );
                return None;
            }
        };
        let (name, name_span) = name;
        if self
            .expect_punct(
                &TokenKind::LBrace,
                "estrutura_forma",
                "'{' após o nome da forma",
            )
            .is_none()
        {
            self.consume_until_comma_or_close();
            return None;
        }

        let mut attrs = FormAttrs::default();
        let mut order: Vec<String> = Vec::new();
        let mut value_expr: Option<Expression> = None;
        let mut horizon: Option<Duration> = None;

        loop {
            if self.is_punct(TokenKind::RBrace) {
                self.advance();
                break;
            }
            match self.peek() {
                None => {
                    self.diags.error(
                        "bloco_nao_fechado",
                        kw_span,
                        format!("forma '{name}' sem '}}'"),
                    );
                    return None;
                }
                Some(Token {
                    kind: TokenKind::Ident(atributo),
                    span,
                }) => {
                    let (atributo, span) = (atributo.clone(), *span);
                    self.advance();
                    self.atributo(
                        &name,
                        conjugation,
                        &atributo,
                        span,
                        &mut attrs,
                        &mut order,
                        &mut value_expr,
                        &mut horizon,
                    );
                }
                Some(t) => {
                    let found = t.text();
                    let span = t.span;
                    self.diags.error(
                        "estrutura_forma",
                        span,
                        format!("esperado nome de atributo (encontrado {found})"),
                    );
                    self.advance();
                    self.consume_until_comma_or_close();
                }
            }
        }

        // Cláusulas por forma (FORMAL §3; Lei 1 do MANIFESTO)
        if !order.contains(&"value".to_string()) {
            self.diags.error(
                "value_obrigatorio",
                kw_span,
                format!("forma '{name}' sem 'value' — obrigatório em toda conjugação (Lei 1)"),
            );
        }
        if !order.contains(&"horizon".to_string()) {
            self.diags.error(
                "horizon_obrigatorio",
                kw_span,
                format!("forma '{name}' sem 'horizon' — obrigatório em toda conjugação (Lei 1)"),
            );
        }
        if let (Some(v), Some(h)) = (
            order.iter().position(|a| a == "value"),
            order.iter().position(|a| a == "horizon"),
        ) {
            if v > h {
                self.diags.error(
                    "ordem_value_horizon",
                    kw_span,
                    format!("forma '{name}': 'value' deve preceder 'horizon' (EBNF: primeiros atributos)"),
                );
            }
        }
        if conjugation == Conjugation::Nonequilibrium && attrs.maintenance_deadline.is_none() {
            self.diags.error(
                "maintenance_deadline_ausente",
                kw_span,
                format!(
                    "forma '{name}': nonequilibrium exige maintenance_deadline — sem ele a forma jamais colapsaria"
                ),
            );
        }
        for (atributo, allowed) in [
            ("maintenance_deadline", Conjugation::Nonequilibrium),
            ("exchange_mode", Conjugation::Nonequilibrium),
            ("cost_bytes", Conjugation::Equilibrium),
        ] {
            let present = match atributo {
                "maintenance_deadline" => attrs.maintenance_deadline.is_some(),
                "exchange_mode" => attrs.exchange_mode.is_some(),
                _ => attrs.cost_bytes.is_some(),
            };
            if present && conjugation != allowed {
                self.diags.error(
                    "atributo_nao_aplicavel",
                    kw_span,
                    format!(
                        "'{atributo}' não se aplica a {} (forma '{name}')",
                        conjugation.name()
                    ),
                );
            }
        }

        let value = value_expr.unwrap_or(Expression::ident("_ausente", kw_span));
        let horizon = horizon.unwrap_or(Duration {
            value: 0.0,
            unit: TimeUnit::S,
            span: kw_span,
        });

        Some(FormDecl {
            conjugation,
            name,
            value,
            horizon,
            attrs,
            span: name_span,
        })
    }

    // ------------------------------------------------------------------
    // optional_attribute (FORMAL §3) — value/horizon tratados à parte
    // ------------------------------------------------------------------
    #[allow(clippy::too_many_arguments)]
    fn atributo(
        &mut self,
        form_name: &str,
        _conjugation: Conjugation,
        name: &str,
        span: Span,
        attrs: &mut FormAttrs,
        order: &mut Vec<String>,
        value_expr: &mut Option<Expression>,
        horizon: &mut Option<Duration>,
    ) {
        if !kw::ATTRIBUTES.contains(&name) {
            self.diags.error(
                "atributo_desconhecido",
                span,
                format!("atributo '{name}' não existe na linguagem"),
            );
            self.consume_until_comma_or_close();
            self.consume_comma("corpo de forma");
            return;
        }
        if order.contains(&name.to_string()) {
            self.diags.error(
                "atributo_duplicado",
                span,
                format!("atributo '{name}' repetido na forma '{form_name}'"),
            );
            self.consume_until_comma_or_close();
            self.consume_comma("corpo de forma");
            return;
        }
        if self
            .expect_punct(
                &TokenKind::Colon,
                "estrutura_forma",
                "':' após o nome do atributo",
            )
            .is_none()
        {
            self.consume_until_comma_or_close();
            self.consume_comma("corpo de forma");
            return;
        }

        match name {
            "value" => {
                *value_expr = Some(self.expression("value"));
                order.push("value".into());
            }
            "horizon" => {
                *horizon = Some(self.duration());
                order.push("horizon".into());
            }
            "source_path" => {
                let expr = self.expression("source_path");
                match expr.kind {
                    ExprKind::Str(s) => {
                        if s.contains('/') || s.starts_with('.') {
                            self.diags.error(
                                "source_path_nao_simbolico",
                                expr.span,
                                format!(
                                    "source_path \"{s}\" deve ser nome simbólico de sensor FXP, nunca caminho de SO"
                                ),
                            );
                        }
                        attrs.source_path = Some(s);
                    }
                    ExprKind::Ident(id) => attrs.source_path = Some(id),
                    ExprKind::Num(_) => {
                        self.diags.error(
                            "estrutura_forma",
                            expr.span,
                            "source_path espera string ou identificador simbólico",
                        );
                    }
                }
                order.push("source_path".into());
            }
            "maintenance_deadline" => {
                attrs.maintenance_deadline = Some(self.duration());
                order.push("maintenance_deadline".into());
            }
            "exchange_mode" => {
                let expr = self.expression("exchange_mode");
                match expr.kind {
                    ExprKind::Str(s) => attrs.exchange_mode = Some(s),
                    ExprKind::Ident(id) => attrs.exchange_mode = Some(id),
                    ExprKind::Num(_) => {
                        self.diags.error(
                            "estrutura_forma",
                            expr.span,
                            "exchange_mode espera string (ex.: \"cooperation\")",
                        );
                    }
                }
                order.push("exchange_mode".into());
            }
            "cost_bytes" => match self.advance() {
                Some(Token {
                    kind: TokenKind::Int(n),
                    span,
                }) => {
                    attrs.cost_bytes = Some(n);
                    order.push("cost_bytes".into());
                    let _ = span;
                }
                Some(Token {
                    kind: TokenKind::Decimal(_),
                    span,
                }) => {
                    self.diags.error(
                        "cost_bytes_inteiro",
                        span,
                        "cost_bytes exige número inteiro (bytes)",
                    );
                    order.push("cost_bytes".into());
                }
                Some(t) => {
                    let found = t.text();
                    self.diags.error(
                        "estrutura_forma",
                        t.span,
                        format!("cost_bytes exige inteiro (encontrado {found})"),
                    );
                }
                None => {
                    let span = self.here();
                    self.diags.error(
                        "estrutura_forma",
                        span,
                        "fim inesperado: cost_bytes exige inteiro",
                    );
                }
            },
            "currency" => {
                let expr = self.expression("currency");
                match expr.kind {
                    ExprKind::Str(s) => attrs.currency = Some(s),
                    ExprKind::Ident(id) => attrs.currency = Some(id),
                    ExprKind::Num(_) => {
                        self.diags.error(
                            "estrutura_forma",
                            expr.span,
                            "currency espera string (ex.: \"CpuCycles\")",
                        );
                    }
                }
                order.push("currency".into());
            }
            "classification" => {
                let expr = self.expression("classification");
                match expr.kind {
                    ExprKind::Str(s) => attrs.classification = Some(s),
                    ExprKind::Ident(id) => attrs.classification = Some(id),
                    ExprKind::Num(_) => {
                        self.diags.error(
                            "estrutura_forma",
                            expr.span,
                            "classification espera string",
                        );
                    }
                }
                order.push("classification".into());
            }
            other => unreachable!("atributo validado acima: {other}"),
        }
        self.consume_comma("corpo de forma");
    }

    // ------------------------------------------------------------------
    // expression = string | number | identifier
    // ------------------------------------------------------------------
    fn expression(&mut self, context: &str) -> Expression {
        match self.advance() {
            Some(Token {
                kind: TokenKind::Str(s),
                span,
            }) => Expression::str(s, span),
            Some(Token {
                kind: TokenKind::Int(n),
                span,
            }) => Expression::num(n as f64, span),
            Some(Token {
                kind: TokenKind::Decimal(x),
                span,
            }) => Expression::num(x, span),
            Some(Token {
                kind: TokenKind::Ident(id),
                span,
            }) => Expression::ident(id, span),
            Some(t) => {
                let found = t.text();
                self.diags.error(
                    "estrutura_forma",
                    t.span,
                    format!("expression inválida para {context} (encontrado {found})"),
                );
                Expression::ident("_invalido", t.span)
            }
            None => {
                let span = self.here();
                self.diags.error(
                    "estrutura_forma",
                    span,
                    format!("fim inesperado: expression exigida em {context}"),
                );
                Expression::ident("_invalido", span)
            }
        }
    }

    // ------------------------------------------------------------------
    // duration = number time_unit
    // ------------------------------------------------------------------
    fn duration(&mut self) -> Duration {
        let span_num = self.peek().map(|t| t.span).unwrap_or_default();
        let value = match self.advance() {
            Some(Token {
                kind: TokenKind::Int(n),
                ..
            }) => n as f64,
            Some(Token {
                kind: TokenKind::Decimal(x),
                ..
            }) => x,
            Some(t) => {
                let found = t.text();
                self.diags.error(
                    "duracao_invalida",
                    t.span,
                    format!("duração esperada: NUM[s|ms|us|ns] (encontrado {found})"),
                );
                self.consume_until_comma_or_close();
                return Duration {
                    value: 0.0,
                    unit: TimeUnit::S,
                    span: span_num,
                };
            }
            None => {
                let span = self.here();
                self.diags
                    .error("duracao_invalida", span, "fim inesperado: duração esperada");
                return Duration {
                    value: 0.0,
                    unit: TimeUnit::S,
                    span,
                };
            }
        };
        match self.advance() {
            Some(Token {
                kind: TokenKind::Ident(u),
                span,
            }) if kw::TIME_UNITS.contains(&u.as_str()) => {
                let unit = match u.as_str() {
                    "s" => TimeUnit::S,
                    "ms" => TimeUnit::Ms,
                    "us" => TimeUnit::Us,
                    _ => TimeUnit::Ns,
                };
                Duration { value, unit, span }
            }
            Some(t) => {
                let found = t.text();
                self.diags.error(
                    "duracao_invalida",
                    t.span,
                    format!("unidade de tempo esperada: s|ms|us|ns (encontrado {found})"),
                );
                Duration {
                    value,
                    unit: TimeUnit::S,
                    span: span_num,
                }
            }
            None => {
                let span = self.here();
                self.diags.error(
                    "duracao_invalida",
                    span,
                    "fim inesperado: unidade de tempo esperada (s|ms|us|ns)",
                );
                Duration {
                    value,
                    unit: TimeUnit::S,
                    span: span_num,
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // review_declaration = 'review' identifier '{' review_rule {',' rule} '}'
    // ------------------------------------------------------------------
    fn review(&mut self, kw_span: Span) -> Option<ReviewDecl> {
        self.advance(); // 'review'
        let name = match self.advance() {
            Some(Token {
                kind: TokenKind::Ident(n),
                span,
            }) => (n, span),
            Some(t) => {
                let found = t.text();
                self.diags.error(
                    "estrutura_review",
                    t.span,
                    format!("esperado identificador após 'review' (encontrado {found})"),
                );
                return None;
            }
            None => {
                self.diags.error(
                    "estrutura_review",
                    kw_span,
                    "fim inesperado: esperado identificador após 'review'",
                );
                return None;
            }
        };
        let (name, _) = name;
        if self
            .expect_punct(
                &TokenKind::LBrace,
                "estrutura_review",
                "'{' após o nome da review",
            )
            .is_none()
        {
            self.consume_until_comma_or_close();
            return None;
        }
        let mut rules = Vec::new();
        loop {
            if self.is_punct(TokenKind::RBrace) {
                self.advance();
                break;
            }
            match self.peek() {
                None => {
                    self.diags.error(
                        "bloco_nao_fechado",
                        kw_span,
                        format!("review '{name}' sem '}}'"),
                    );
                    return None;
                }
                Some(Token {
                    kind: TokenKind::Ident(s),
                    ..
                }) if s == "when" => {
                    match self.rule() {
                        Some(r) => rules.push(r),
                        None => self.consume_until_comma_or_close(),
                    }
                    self.consume_comma("review");
                }
                Some(t) => {
                    let found = t.text();
                    let span = t.span;
                    self.diags.error(
                        "regra_mal_formada",
                        span,
                        format!("regra deve começar com 'when' (encontrado {found})"),
                    );
                    self.advance();
                    self.consume_until_comma_or_close();
                    self.consume_comma("review");
                }
            }
        }
        Some(ReviewDecl {
            form: name,
            rules,
            span: kw_span,
        })
    }

    /// review_rule = 'when' sensor_ref comparison_op threshold '->' action_list
    fn rule(&mut self) -> Option<Rule> {
        let when_span = self.peek().unwrap().span;
        self.advance(); // 'when'
        let sensor = match self.advance() {
            Some(Token {
                kind: TokenKind::Ident(s),
                span,
            }) => SensorRef { name: s, span },
            Some(Token {
                kind: TokenKind::Str(s),
                span,
            }) => SensorRef { name: s, span },
            Some(t) => {
                let found = t.text();
                self.diags.error(
                    "regra_mal_formada",
                    t.span,
                    format!("esperado sensor (identificador ou string) após 'when' (encontrado {found})"),
                );
                return None;
            }
            None => {
                self.diags.error(
                    "regra_mal_formada",
                    when_span,
                    "fim inesperado: sensor esperado após 'when'",
                );
                return None;
            }
        };
        let op = match self.advance() {
            Some(Token {
                kind: TokenKind::Lt,
                ..
            }) => CmpOp::Lt,
            Some(Token {
                kind: TokenKind::Gt,
                ..
            }) => CmpOp::Gt,
            Some(Token {
                kind: TokenKind::Le,
                ..
            }) => CmpOp::Le,
            Some(Token {
                kind: TokenKind::Ge,
                ..
            }) => CmpOp::Ge,
            Some(Token {
                kind: TokenKind::EqEq,
                ..
            }) => CmpOp::Eq,
            Some(Token {
                kind: TokenKind::NotEq,
                ..
            }) => CmpOp::Ne,
            Some(t) => {
                let found = t.text();
                self.diags.error(
                    "operador_invalido",
                    t.span,
                    format!("operador de comparação inválido: {found} (use < > <= >= == !=)"),
                );
                return None;
            }
            None => {
                self.diags.error(
                    "operador_invalido",
                    when_span,
                    "fim inesperado: operador de comparação esperado",
                );
                return None;
            }
        };
        let threshold = self.threshold()?;
        self.expect_punct(
            &TokenKind::Arrow,
            "regra_mal_formada",
            "'->' antes das ações",
        )?;
        let actions = self.actions()?;
        Some(Rule {
            sensor,
            op,
            threshold,
            actions,
            span: when_span,
        })
    }

    /// threshold = number | percentage | physical_quantity
    fn threshold(&mut self) -> Option<Threshold> {
        let (value, _span) = match self.advance() {
            Some(Token {
                kind: TokenKind::Int(n),
                span,
            }) => (n as f64, span),
            Some(Token {
                kind: TokenKind::Decimal(x),
                span,
            }) => (x, span),
            Some(t) => {
                let found = t.text();
                self.diags.error(
                    "regra_mal_formada",
                    t.span,
                    format!("threshold deve ser número (encontrado {found})"),
                );
                return None;
            }
            None => {
                self.diags.error(
                    "regra_mal_formada",
                    self.here(),
                    "fim inesperado: threshold deve ser número",
                );
                return None;
            }
        };
        // Unidade opcional: `%` e `°C` têm token próprio; `W` é identificador.
        let unit = match self.peek_kind() {
            Some(TokenKind::Percent) => {
                self.advance();
                Some(PhysicalUnit::Percent)
            }
            Some(TokenKind::DegC) => {
                self.advance();
                Some(PhysicalUnit::DegC)
            }
            Some(TokenKind::Ident(s)) if s == kw::W => {
                self.advance();
                Some(PhysicalUnit::W)
            }
            _ => None,
        };
        Some(Threshold { value, unit })
    }

    /// action_list = action { ',' action }
    fn actions(&mut self) -> Option<Vec<Action>> {
        let mut actions = Vec::new();
        loop {
            let action = match self.advance() {
                Some(Token {
                    kind: TokenKind::Ident(s),
                    span,
                }) => match s.as_str() {
                    "dissolve" => Action::Dissolve,
                    "subvert" => Action::Subvert,
                    "reclassify_as_equilibrium" => Action::ReclassifyAsEquilibrium,
                    "reclassify_as_nonequilibrium" => Action::ReclassifyAsNonequilibrium,
                    "notify_shutdown" => Action::NotifyShutdown,
                    "act" => {
                        self.expect_punct(&TokenKind::LParen, "regra_mal_formada", "'(' no act")?;
                        let (actor, actor_span) = match self.advance() {
                            Some(Token {
                                kind: TokenKind::Ident(n),
                                span,
                            }) => (n, span),
                            Some(Token {
                                kind: TokenKind::Str(n),
                                span,
                            }) => (n, span),
                            Some(t) => {
                                let found = t.text();
                                self.diags.error(
                                    "regra_mal_formada",
                                    t.span,
                                    format!("ator esperado no act (encontrado {found})"),
                                );
                                return None;
                            }
                            None => {
                                self.diags.error(
                                    "regra_mal_formada",
                                    self.here(),
                                    "fim inesperado: ator esperado no act",
                                );
                                return None;
                            }
                        };
                        self.expect_punct(&TokenKind::Comma, "regra_mal_formada", "',' no act")?;
                        let value = self.expression("act");
                        self.expect_punct(&TokenKind::RParen, "regra_mal_formada", "')' no act")?;
                        Action::Act {
                            actor,
                            value,
                            actor_span,
                        }
                    }
                    other => {
                        self.diags.error(
                            "acao_desconhecida",
                            span,
                            format!("ação '{other}' desconhecida na action_list"),
                        );
                        self.consume_until_comma_or_close();
                        return None;
                    }
                },
                Some(t) => {
                    let found = t.text();
                    self.diags.error(
                        "acao_desconhecida",
                        t.span,
                        format!("ação desconhecida na action_list (encontrado {found})"),
                    );
                    self.consume_until_comma_or_close();
                    return None;
                }
                None => {
                    self.diags.error(
                        "acao_desconhecida",
                        self.here(),
                        "fim inesperado: action_list vazia",
                    );
                    return None;
                }
            };
            actions.push(action);
            if self.is_punct(TokenKind::Comma) {
                // vírgula entre ações; se o próximo token não é outra ação,
                // devolve o controle (vírgula final/vírgula entre regras)
                self.advance();
                match self.peek_kind() {
                    Some(TokenKind::Ident(s)) if kw::ACTIONS.contains(&s.as_str()) => continue,
                    Some(TokenKind::RBrace) => {
                        let span = self.peek().unwrap().span;
                        self.diags.error(
                            "virgula_final",
                            span,
                            "vírgula final antes de '}' em review (EBNF não permite)",
                        );
                        return Some(actions);
                    }
                    _ => return Some(actions),
                }
            }
            return Some(actions);
        }
    }

    // ------------------------------------------------------------------
    // main_block = 'main' '{' statement {',' statement} '}'
    // ------------------------------------------------------------------
    fn main_block(&mut self, kw_span: Span) -> Option<MainBlock> {
        self.advance(); // 'main'
        if self
            .expect_punct(&TokenKind::LBrace, "estrutura_main", "'{' após 'main'")
            .is_none()
        {
            self.consume_until_comma_or_close();
            return None;
        }
        let mut statements = Vec::new();
        loop {
            if self.is_punct(TokenKind::RBrace) {
                self.advance();
                break;
            }
            match self.peek() {
                None => {
                    self.diags
                        .error("bloco_nao_fechado", kw_span, "main sem '}'");
                    return None;
                }
                Some(_) => {
                    match self.statement(0) {
                        Some(s) => statements.push(s),
                        None => self.consume_until_comma_or_close(),
                    }
                    self.consume_comma("main");
                }
            }
        }
        Some(MainBlock {
            statements,
            span: kw_span,
        })
    }

    /// statement = keep '(' identifier ')' | act '(' ator ',' expression ')'
    ///           | every duration '{' statement {',' statement} '}'
    fn statement(&mut self, depth: usize) -> Option<Statement> {
        match self.advance()? {
            Token {
                kind: TokenKind::Ident(s),
                span,
            } => match s.as_str() {
                "keep" => {
                    self.expect_punct(&TokenKind::LParen, "estrutura_main", "'(' no keep")?;
                    let form = match self.advance() {
                        Some(Token {
                            kind: TokenKind::Ident(n),
                            ..
                        }) => n,
                        Some(t) => {
                            let found = t.text();
                            self.diags.error(
                                "estrutura_main",
                                t.span,
                                format!("forma esperada no keep (encontrado {found})"),
                            );
                            return None;
                        }
                        None => {
                            self.diags.error(
                                "estrutura_main",
                                self.here(),
                                "fim inesperado: forma esperada no keep",
                            );
                            return None;
                        }
                    };
                    self.expect_punct(&TokenKind::RParen, "estrutura_main", "')' no keep")?;
                    Some(Statement::Keep(form))
                }
                "act" => {
                    self.expect_punct(&TokenKind::LParen, "estrutura_main", "'(' no act")?;
                    let actor = match self.advance() {
                        Some(Token {
                            kind: TokenKind::Ident(n),
                            ..
                        })
                        | Some(Token {
                            kind: TokenKind::Str(n),
                            ..
                        }) => n,
                        Some(t) => {
                            let found = t.text();
                            self.diags.error(
                                "estrutura_main",
                                t.span,
                                format!("ator esperado no act (encontrado {found})"),
                            );
                            return None;
                        }
                        None => {
                            self.diags.error(
                                "estrutura_main",
                                self.here(),
                                "fim inesperado: ator esperado no act",
                            );
                            return None;
                        }
                    };
                    self.expect_punct(&TokenKind::Comma, "estrutura_main", "',' no act")?;
                    let value = self.expression("act");
                    self.expect_punct(&TokenKind::RParen, "estrutura_main", "')' no act")?;
                    Some(Statement::Act { actor, value })
                }
                "every" => {
                    if depth >= MAX_EVERY_DEPTH {
                        self.diags.error(
                            "every_muito_profundo",
                            span,
                            format!("aninhamento de 'every' além de {MAX_EVERY_DEPTH} níveis"),
                        );
                        return None;
                    }
                    let period = self.duration();
                    self.expect_punct(&TokenKind::LBrace, "estrutura_main", "'{' no every")?;
                    let mut body = Vec::new();
                    loop {
                        if self.is_punct(TokenKind::RBrace) {
                            self.advance();
                            break;
                        }
                        match self.peek() {
                            None => {
                                self.diags.error("bloco_nao_fechado", span, "every sem '}'");
                                return None;
                            }
                            Some(_) => {
                                if let Some(st) = self.statement(depth + 1) {
                                    body.push(st);
                                } else {
                                    self.consume_until_comma_or_close();
                                }
                                self.consume_comma("every");
                            }
                        }
                    }
                    Some(Statement::Every { period, body })
                }
                other => {
                    self.diags.error(
                        "statement_desconhecido",
                        span,
                        format!("statement '{other}' não existe no main (keep|act|every)"),
                    );
                    None
                }
            },
            t => {
                let found = t.text();
                self.diags.error(
                    "statement_desconhecido",
                    t.span,
                    format!("statement inválido no main (encontrado {found})"),
                );
                None
            }
        }
    }

    // ------------------------------------------------------------------
    // Cláusulas cruzadas entre declarações (FORMAL §3)
    // ------------------------------------------------------------------
    fn cross_clauses(&mut self, program: &Program) {
        let mut forms: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut reviews_seen: std::collections::BTreeMap<&str, u32> =
            std::collections::BTreeMap::new();

        // 1ª passada: conjunto de formas declaradas (a ligação review→forma
        // é independente da ordem das declarações — EBNF permite intercalar)
        for decl in &program.decls {
            if let Declaration::Form(f) = decl {
                if !forms.insert(f.name.as_str()) {
                    self.diags.error(
                        "forma_duplicada",
                        f.span,
                        format!(
                            "forma '{}' declarada duas vezes — regras não são mescladas",
                            f.name
                        ),
                    );
                }
            }
        }

        // 2ª passada: reviews órfãs/duplicadas
        for decl in &program.decls {
            if let Declaration::Review(r) = decl {
                if !forms.contains(r.form.as_str()) {
                    self.diags.error(
                        "review_orfa",
                        r.span,
                        format!(
                            "review para forma inexistente: '{}' — erro de compilação (FORMAL §3)",
                            r.form
                        ),
                    );
                }
                let seen = reviews_seen.entry(r.form.as_str()).or_insert(0);
                *seen += 1;
                if *seen > 1 {
                    self.diags.error(
                        "review_duplicada",
                        r.span,
                        format!(
                            "segunda review para '{}' — regras não são mescladas (FORMAL §3)",
                            r.form
                        ),
                    );
                }
            }
        }

        // keep de forma inexistente (cláusula de erro — AGENTS.md §2.2)
        if let Some(main) = &program.main {
            fn pass(
                stmts: &[Statement],
                forms: &std::collections::BTreeSet<&str>,
                diags: &mut Diagnostics,
                main_span: Span,
            ) {
                for st in stmts {
                    match st {
                        Statement::Keep(name) => {
                            if !forms.contains(name.as_str()) {
                                diags.error(
                                    "keep_forma_inexistente",
                                    main_span,
                                    format!(
                                        "keep('{name}') não aponta para forma declarada — cláusula de erro"
                                    ),
                                );
                            }
                        }
                        Statement::Every { body, .. } => pass(body, forms, diags, main_span),
                        Statement::Act { .. } => {}
                    }
                }
            }
            pass(&main.statements, &forms, &mut self.diags, main.span);
        }
    }
}
