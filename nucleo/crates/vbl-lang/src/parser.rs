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
pub fn parse(fonte: &str) -> (Program, Diagnostics) {
    let (tokens, mut diags) = tokenize(fonte);
    let mut p = Parser { toks: tokens, pos: 0, diags: Diagnostics::new() };
    let programa = p.programa();
    p.clausulas_cruzadas(&programa);
    diags.extend(p.diags);
    diags.sort();
    (programa, diags)
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

    fn avancar(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn aqui(&self) -> Span {
        self.toks.last().map(|t| t.span).unwrap_or_default()
    }

    /// Consome o token esperado ou registra `codigo`.
    fn esperar_punc(&mut self, kind: &TokenKind, codigo: &str, msg: &str) -> Option<Token> {
        match self.peek() {
            Some(t) if &t.kind == kind => self.avancar(),
            Some(t) => {
                let found = t.texto();
                let span = t.span;
                self.diags
                    .error(codigo, span, format!("{msg} (encontrado {found})"));
                None
            }
            None => {
                let span = self.aqui();
                self.diags.error(codigo, span, format!("fim inesperado: {msg}"));
                None
            }
        }
    }

    /// Consome `,` se presente; registra vírgula final antes de `}`.
    fn consumir_virgula(&mut self, contexto: &str) {
        if self.eh_punc(TokenKind::Comma) {
            self.avancar();
            if self.eh_punc(TokenKind::RBrace) {
                let span = self.peek().unwrap().span;
                self.diags.error(
                    "virgula_final",
                    span,
                    format!("vírgula final antes de '}}' em {contexto} (EBNF não permite)"),
                );
            }
        }
    }

    fn eh_punc(&self, kind: TokenKind) -> bool {
        matches!(self.peek_kind(), Some(k) if *k == kind)
    }

    /// Descarta tokens até vírgula ou fechamento de bloco (recuperação).
    fn consumir_ate_virgula_ou_fecha(&mut self) {
        let mut profundeza = 0usize;
        while let Some(t) = self.peek() {
            match t.kind {
                TokenKind::LBrace | TokenKind::LParen => profundeza += 1,
                TokenKind::RBrace => {
                    if profundeza == 0 {
                        return;
                    }
                    profundeza -= 1;
                }
                TokenKind::RParen => {
                    if profundeza == 0 {
                        return;
                    }
                    profundeza -= 1;
                }
                TokenKind::Comma if profundeza == 0 => return,
                _ => {}
            }
            self.avancar();
        }
    }

    // ------------------------------------------------------------------
    // program = { form_declaration | review_declaration } [ main_block ]
    // ------------------------------------------------------------------
    fn programa(&mut self) -> Program {
        let mut programa = Program::default();
        let mut main_visto: Option<Span> = None;
        while let Some(tok) = self.peek().cloned() {
            let TokenKind::Ident(nome) = &tok.kind else {
                let found = tok.texto();
                let span = tok.span;
                self.diags.error(
                    "topo_invalido",
                    span,
                    format!("esperada declaração (forma/review/main), encontrado {found}"),
                );
                self.avancar();
                continue;
            };
            match nome.as_str() {
                "event" | "equilibrium" | "nonequilibrium" => {
                    if main_visto.is_some() {
                        self.diags.error(
                            "main_deve_ser_ultimo",
                            tok.span,
                            "declaração após o bloco `main` (EBNF: main é o último item)",
                        );
                    }
                    if let Some(f) = self.forma(tok.span) {
                        programa.decls.push(Declaration::Form(Box::new(f)));
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
                        programa.decls.push(Declaration::Review(r));
                    }
                }
                "main" => {
                    if let Some(anterior) = main_visto {
                        self.diags.error(
                            "main_duplicado",
                            tok.span,
                            format!("segundo bloco `main` (o primeiro está em {})", anterior),
                        );
                        self.avancar();
                        self.consumir_ate_virgula_ou_fecha();
                        continue;
                    }
                    main_visto = Some(tok.span);
                    if let Some(m) = self.bloco_main(tok.span) {
                        programa.main = Some(m);
                    }
                }
                outra => {
                    self.diags.error(
                        "topo_invalido",
                        tok.span,
                        format!("declaração desconhecida '{outra}' (esperado forma, review ou main)"),
                    );
                    self.avancar();
                }
            }
        }
        programa
    }

    // ------------------------------------------------------------------
    // form_declaration = conjugation_kw identifier '{' form_body '}'
    // ------------------------------------------------------------------
    fn forma(&mut self, kw_span: Span) -> Option<FormDecl> {
        let conjugation = match self.peek_kind() {
            Some(TokenKind::Ident(s)) => match s.as_str() {
                "event" => {
                    self.avancar();
                    Conjugation::Event
                }
                "equilibrium" => {
                    self.avancar();
                    Conjugation::Equilibrium
                }
                "nonequilibrium" => {
                    self.avancar();
                    Conjugation::Nonequilibrium
                }
                _ => unreachable!("chamado apenas com conjugação"),
            },
            _ => unreachable!(),
        };
        let nome = match self.avancar() {
            Some(Token { kind: TokenKind::Ident(n), span }) => (n, span),
            Some(t) => {
                let found = t.texto();
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
        let (nome, nome_span) = nome;
        if self.esperar_punc(&TokenKind::LBrace, "estrutura_forma", "'{' após o nome da forma")
            .is_none()
        {
            self.consumir_ate_virgula_ou_fecha();
            return None;
        }

        let mut attrs = FormAttrs::default();
        let mut ordem: Vec<String> = Vec::new();
        let mut value_expr: Option<Expression> = None;
        let mut horizon: Option<Duration> = None;

        loop {
            if self.eh_punc(TokenKind::RBrace) {
                self.avancar();
                break;
            }
            match self.peek() {
                None => {
                    self.diags.error(
                        "bloco_nao_fechado",
                        kw_span,
                        format!("forma '{nome}' sem '}}'"),
                    );
                    return None;
                }
                Some(Token { kind: TokenKind::Ident(atributo), span }) => {
                    let (atributo, span) = (atributo.clone(), *span);
                    self.avancar();
                    self.atributo(
                        &nome,
                        conjugation,
                        &atributo,
                        span,
                        &mut attrs,
                        &mut ordem,
                        &mut value_expr,
                        &mut horizon,
                    );
                }
                Some(t) => {
                    let found = t.texto();
                    let span = t.span;
                    self.diags.error(
                        "estrutura_forma",
                        span,
                        format!("esperado nome de atributo (encontrado {found})"),
                    );
                    self.avancar();
                    self.consumir_ate_virgula_ou_fecha();
                }
            }
        }

        // Cláusulas por forma (FORMAL §3; Lei 1 do MANIFESTO)
        if !ordem.contains(&"value".to_string()) {
            self.diags.error(
                "value_obrigatorio",
                kw_span,
                format!("forma '{nome}' sem 'value' — obrigatório em toda conjugação (Lei 1)"),
            );
        }
        if !ordem.contains(&"horizon".to_string()) {
            self.diags.error(
                "horizon_obrigatorio",
                kw_span,
                format!("forma '{nome}' sem 'horizon' — obrigatório em toda conjugação (Lei 1)"),
            );
        }
        if let (Some(v), Some(h)) = (ordem.iter().position(|a| a == "value"), ordem.iter().position(|a| a == "horizon")) {
            if v > h {
                self.diags.error(
                    "ordem_value_horizon",
                    kw_span,
                    format!("forma '{nome}': 'value' deve preceder 'horizon' (EBNF: primeiros atributos)"),
                );
            }
        }
        if conjugation == Conjugation::Nonequilibrium && attrs.maintenance_deadline.is_none() {
            self.diags.error(
                "maintenance_deadline_ausente",
                kw_span,
                format!(
                    "forma '{nome}': nonequilibrium exige maintenance_deadline — sem ele a forma jamais colapsaria"
                ),
            );
        }
        for (atributo, permitidas) in [
            ("maintenance_deadline", Conjugation::Nonequilibrium),
            ("exchange_mode", Conjugation::Nonequilibrium),
            ("cost_bytes", Conjugation::Equilibrium),
        ] {
            let presente = match atributo {
                "maintenance_deadline" => attrs.maintenance_deadline.is_some(),
                "exchange_mode" => attrs.exchange_mode.is_some(),
                _ => attrs.cost_bytes.is_some(),
            };
            if presente && conjugation != permitidas {
                self.diags.error(
                    "atributo_nao_aplicavel",
                    kw_span,
                    format!(
                        "'{atributo}' não se aplica a {} (forma '{nome}')",
                        conjugation.nome()
                    ),
                );
            }
        }

        let value = value_expr.unwrap_or(Expression::ident("_ausente", kw_span));
        let horizon = horizon.unwrap_or(Duration {
            valor: 0.0,
            unit: TimeUnit::S,
            span: kw_span,
        });

        Some(FormDecl { conjugation, name: nome, value, horizon, attrs, span: nome_span })
    }

    // ------------------------------------------------------------------
    // optional_attribute (FORMAL §3) — value/horizon tratados à parte
    // ------------------------------------------------------------------
    #[allow(clippy::too_many_arguments)]
    fn atributo(
        &mut self,
        nome_forma: &str,
        _conjugation: Conjugation,
        nome: &str,
        span: Span,
        attrs: &mut FormAttrs,
        ordem: &mut Vec<String>,
        value_expr: &mut Option<Expression>,
        horizon: &mut Option<Duration>,
    ) {
        if !kw::ATRIBUTOS.contains(&nome) {
            self.diags.error(
                "atributo_desconhecido",
                span,
                format!("atributo '{nome}' não existe na linguagem"),
            );
            self.consumir_ate_virgula_ou_fecha();
            self.consumir_virgula("corpo de forma");
            return;
        }
        if ordem.contains(&nome.to_string()) {
            self.diags.error(
                "atributo_duplicado",
                span,
                format!("atributo '{nome}' repetido na forma '{nome_forma}'"),
            );
            self.consumir_ate_virgula_ou_fecha();
            self.consumir_virgula("corpo de forma");
            return;
        }
        if self
            .esperar_punc(&TokenKind::Colon, "estrutura_forma", "':' após o nome do atributo")
            .is_none()
        {
            self.consumir_ate_virgula_ou_fecha();
            self.consumir_virgula("corpo de forma");
            return;
        }

        match nome {
            "value" => {
                *value_expr = Some(self.expression("value"));
                ordem.push("value".into());
            }
            "horizon" => {
                *horizon = Some(self.duracao());
                ordem.push("horizon".into());
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
                ordem.push("source_path".into());
            }
            "maintenance_deadline" => {
                attrs.maintenance_deadline = Some(self.duracao());
                ordem.push("maintenance_deadline".into());
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
                ordem.push("exchange_mode".into());
            }
            "cost_bytes" => match self.avancar() {
                Some(Token { kind: TokenKind::Int(n), span }) => {
                    attrs.cost_bytes = Some(n);
                    ordem.push("cost_bytes".into());
                    let _ = span;
                }
                Some(Token { kind: TokenKind::Decimal(_), span }) => {
                    self.diags.error(
                        "cost_bytes_inteiro",
                        span,
                        "cost_bytes exige número inteiro (bytes)",
                    );
                    ordem.push("cost_bytes".into());
                }
                Some(t) => {
                    let found = t.texto();
                    self.diags.error(
                        "estrutura_forma",
                        t.span,
                        format!("cost_bytes exige inteiro (encontrado {found})"),
                    );
                }
                None => {
                    let span = self.aqui();
                    self.diags
                        .error("estrutura_forma", span, "fim inesperado: cost_bytes exige inteiro");
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
                ordem.push("currency".into());
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
                ordem.push("classification".into());
            }
            outro => unreachable!("atributo validado acima: {outro}"),
        }
        self.consumir_virgula("corpo de forma");
    }

    // ------------------------------------------------------------------
    // expression = string | number | identifier
    // ------------------------------------------------------------------
    fn expression(&mut self, contexto: &str) -> Expression {
        match self.avancar() {
            Some(Token { kind: TokenKind::Str(s), span }) => Expression::str(s, span),
            Some(Token { kind: TokenKind::Int(n), span }) => Expression::num(n as f64, span),
            Some(Token { kind: TokenKind::Decimal(x), span }) => Expression::num(x, span),
            Some(Token { kind: TokenKind::Ident(id), span }) => Expression::ident(id, span),
            Some(t) => {
                let found = t.texto();
                self.diags.error(
                    "estrutura_forma",
                    t.span,
                    format!("expression inválida para {contexto} (encontrado {found})"),
                );
                Expression::ident("_invalido", t.span)
            }
            None => {
                let span = self.aqui();
                self.diags.error(
                    "estrutura_forma",
                    span,
                    format!("fim inesperado: expression exigida em {contexto}"),
                );
                Expression::ident("_invalido", span)
            }
        }
    }

    // ------------------------------------------------------------------
    // duration = number time_unit
    // ------------------------------------------------------------------
    fn duracao(&mut self) -> Duration {
        let span_num = self.peek().map(|t| t.span).unwrap_or_default();
        let valor = match self.avancar() {
            Some(Token { kind: TokenKind::Int(n), .. }) => n as f64,
            Some(Token { kind: TokenKind::Decimal(x), .. }) => x,
            Some(t) => {
                let found = t.texto();
                self.diags.error(
                    "duracao_invalida",
                    t.span,
                    format!("duração esperada: NUM[s|ms|us|ns] (encontrado {found})"),
                );
                self.consumir_ate_virgula_ou_fecha();
                return Duration { valor: 0.0, unit: TimeUnit::S, span: span_num };
            }
            None => {
                let span = self.aqui();
                self.diags
                    .error("duracao_invalida", span, "fim inesperado: duração esperada");
                return Duration { valor: 0.0, unit: TimeUnit::S, span };
            }
        };
        match self.avancar() {
            Some(Token { kind: TokenKind::Ident(u), span }) if kw::UNIDADES_TEMPO.contains(&u.as_str()) => {
                let unit = match u.as_str() {
                    "s" => TimeUnit::S,
                    "ms" => TimeUnit::Ms,
                    "us" => TimeUnit::Us,
                    _ => TimeUnit::Ns,
                };
                Duration { valor, unit, span }
            }
            Some(t) => {
                let found = t.texto();
                self.diags.error(
                    "duracao_invalida",
                    t.span,
                    format!("unidade de tempo esperada: s|ms|us|ns (encontrado {found})"),
                );
                Duration { valor, unit: TimeUnit::S, span: span_num }
            }
            None => {
                let span = self.aqui();
                self.diags.error(
                    "duracao_invalida",
                    span,
                    "fim inesperado: unidade de tempo esperada (s|ms|us|ns)",
                );
                Duration { valor, unit: TimeUnit::S, span: span_num }
            }
        }
    }

    // ------------------------------------------------------------------
    // review_declaration = 'review' identifier '{' review_rule {',' rule} '}'
    // ------------------------------------------------------------------
    fn review(&mut self, kw_span: Span) -> Option<ReviewDecl> {
        self.avancar(); // 'review'
        let nome = match self.avancar() {
            Some(Token { kind: TokenKind::Ident(n), span }) => (n, span),
            Some(t) => {
                let found = t.texto();
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
        let (nome, _) = nome;
        if self
            .esperar_punc(&TokenKind::LBrace, "estrutura_review", "'{' após o nome da review")
            .is_none()
        {
            self.consumir_ate_virgula_ou_fecha();
            return None;
        }
        let mut rules = Vec::new();
        loop {
            if self.eh_punc(TokenKind::RBrace) {
                self.avancar();
                break;
            }
            match self.peek() {
                None => {
                    self.diags.error("bloco_nao_fechado", kw_span, format!("review '{nome}' sem '}}'"));
                    return None;
                }
                Some(Token { kind: TokenKind::Ident(s), .. }) if s == "when" => {
                    match self.regra() {
                        Some(r) => rules.push(r),
                        None => self.consumir_ate_virgula_ou_fecha(),
                    }
                    self.consumir_virgula("review");
                }
                Some(t) => {
                    let found = t.texto();
                    let span = t.span;
                    self.diags.error(
                        "regra_mal_formada",
                        span,
                        format!("regra deve começar com 'when' (encontrado {found})"),
                    );
                    self.avancar();
                    self.consumir_ate_virgula_ou_fecha();
                    self.consumir_virgula("review");
                }
            }
        }
        Some(ReviewDecl { form: nome, rules, span: kw_span })
    }

    /// review_rule = 'when' sensor_ref comparison_op threshold '->' action_list
    fn regra(&mut self) -> Option<Rule> {
        let when_span = self.peek().unwrap().span;
        self.avancar(); // 'when'
        let sensor = match self.avancar() {
            Some(Token { kind: TokenKind::Ident(s), span }) => SensorRef { nome: s, span },
            Some(Token { kind: TokenKind::Str(s), span }) => SensorRef { nome: s, span },
            Some(t) => {
                let found = t.texto();
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
        let op = match self.avancar() {
            Some(Token { kind: TokenKind::Lt, .. }) => CmpOp::Lt,
            Some(Token { kind: TokenKind::Gt, .. }) => CmpOp::Gt,
            Some(Token { kind: TokenKind::Le, .. }) => CmpOp::Le,
            Some(Token { kind: TokenKind::Ge, .. }) => CmpOp::Ge,
            Some(Token { kind: TokenKind::EqEq, .. }) => CmpOp::Eq,
            Some(Token { kind: TokenKind::NotEq, .. }) => CmpOp::Ne,
            Some(t) => {
                let found = t.texto();
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
        self.esperar_punc(&TokenKind::Arrow, "regra_mal_formada", "'->' antes das ações")?;
        let actions = self.acoes()?;
        Some(Rule { sensor, op, threshold, actions, span: when_span })
    }

    /// threshold = number | percentage | physical_quantity
    fn threshold(&mut self) -> Option<Threshold> {
        let (valor, _span) = match self.avancar() {
            Some(Token { kind: TokenKind::Int(n), span }) => (n as f64, span),
            Some(Token { kind: TokenKind::Decimal(x), span }) => (x, span),
            Some(t) => {
                let found = t.texto();
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
                    self.aqui(),
                    "fim inesperado: threshold deve ser número",
                );
                return None;
            }
        };
        // Unidade opcional: `%` e `°C` têm token próprio; `W` é identificador.
        let unit = match self.peek_kind() {
            Some(TokenKind::Percent) => {
                self.avancar();
                Some(PhysicalUnit::Percent)
            }
            Some(TokenKind::DegC) => {
                self.avancar();
                Some(PhysicalUnit::DegC)
            }
            Some(TokenKind::Ident(s)) if s == kw::W => {
                self.avancar();
                Some(PhysicalUnit::W)
            }
            _ => None,
        };
        Some(Threshold { valor, unit })
    }

    /// action_list = action { ',' action }
    fn acoes(&mut self) -> Option<Vec<Action>> {
        let mut acoes = Vec::new();
        loop {
            let acao = match self.avancar() {
                Some(Token { kind: TokenKind::Ident(s), span }) => match s.as_str() {
                    "dissolve" => Action::Dissolve,
                    "subvert" => Action::Subvert,
                    "reclassify_as_equilibrium" => Action::ReclassifyAsEquilibrium,
                    "reclassify_as_nonequilibrium" => Action::ReclassifyAsNonequilibrium,
                    "notify_shutdown" => Action::NotifyShutdown,
                    "act" => {
                        self.esperar_punc(&TokenKind::LParen, "regra_mal_formada", "'(' no act")?;
                        let (actor, actor_span) = match self.avancar() {
                            Some(Token { kind: TokenKind::Ident(n), span }) => (n, span),
                            Some(Token { kind: TokenKind::Str(n), span }) => (n, span),
                            Some(t) => {
                                let found = t.texto();
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
                                    self.aqui(),
                                    "fim inesperado: ator esperado no act",
                                );
                                return None;
                            }
                        };
                        self.esperar_punc(&TokenKind::Comma, "regra_mal_formada", "',' no act")?;
                        let value = self.expression("act");
                        self
                            .esperar_punc(&TokenKind::RParen, "regra_mal_formada", "')' no act")?;
                        Action::Act { actor, value, actor_span }
                    }
                    outra => {
                        self.diags.error(
                            "acao_desconhecida",
                            span,
                            format!("ação '{outra}' desconhecida na action_list"),
                        );
                        self.consumir_ate_virgula_ou_fecha();
                        return None;
                    }
                },
                Some(t) => {
                    let found = t.texto();
                    self.diags.error(
                        "acao_desconhecida",
                        t.span,
                        format!("ação desconhecida na action_list (encontrado {found})"),
                    );
                    self.consumir_ate_virgula_ou_fecha();
                    return None;
                }
                None => {
                    self.diags.error(
                        "acao_desconhecida",
                        self.aqui(),
                        "fim inesperado: action_list vazia",
                    );
                    return None;
                }
            };
            acoes.push(acao);
            if self.eh_punc(TokenKind::Comma) {
                // vírgula entre ações; se o próximo token não é outra ação,
                // devolve o controle (vírgula final/vírgula entre regras)
                self.avancar();
                match self.peek_kind() {
                    Some(TokenKind::Ident(s)) if kw::ACOES.contains(&s.as_str()) => continue,
                    Some(TokenKind::RBrace) => {
                        let span = self.peek().unwrap().span;
                        self.diags.error(
                            "virgula_final",
                            span,
                            "vírgula final antes de '}' em review (EBNF não permite)",
                        );
                        return Some(acoes);
                    }
                    _ => return Some(acoes),
                }
            }
            return Some(acoes);
        }
    }

    // ------------------------------------------------------------------
    // main_block = 'main' '{' statement {',' statement} '}'
    // ------------------------------------------------------------------
    fn bloco_main(&mut self, kw_span: Span) -> Option<MainBlock> {
        self.avancar(); // 'main'
        if self.esperar_punc(&TokenKind::LBrace, "estrutura_main", "'{' após 'main'").is_none() {
            self.consumir_ate_virgula_ou_fecha();
            return None;
        }
        let mut statements = Vec::new();
        loop {
            if self.eh_punc(TokenKind::RBrace) {
                self.avancar();
                break;
            }
            match self.peek() {
                None => {
                    self.diags.error("bloco_nao_fechado", kw_span, "main sem '}'");
                    return None;
                }
                Some(_) => {
                    match self.statement(0) {
                        Some(s) => statements.push(s),
                        None => self.consumir_ate_virgula_ou_fecha(),
                    }
                    self.consumir_virgula("main");
                }
            }
        }
        Some(MainBlock { statements, span: kw_span })
    }

    /// statement = keep '(' identifier ')' | act '(' ator ',' expression ')'
    ///           | every duration '{' statement {',' statement} '}'
    fn statement(&mut self, profundeza: usize) -> Option<Statement> {
        match self.avancar()? {
            Token { kind: TokenKind::Ident(s), span } => match s.as_str() {
                "keep" => {
                    self.esperar_punc(&TokenKind::LParen, "estrutura_main", "'(' no keep")?;
                    let forma = match self.avancar() {
                        Some(Token { kind: TokenKind::Ident(n), .. }) => n,
                        Some(t) => {
                            let found = t.texto();
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
                                self.aqui(),
                                "fim inesperado: forma esperada no keep",
                            );
                            return None;
                        }
                    };
                    self.esperar_punc(&TokenKind::RParen, "estrutura_main", "')' no keep")?;
                    Some(Statement::Keep(forma))
                }
                "act" => {
                    self.esperar_punc(&TokenKind::LParen, "estrutura_main", "'(' no act")?;
                    let actor = match self.avancar() {
                        Some(Token { kind: TokenKind::Ident(n), .. })
                        | Some(Token { kind: TokenKind::Str(n), .. }) => n,
                        Some(t) => {
                            let found = t.texto();
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
                                self.aqui(),
                                "fim inesperado: ator esperado no act",
                            );
                            return None;
                        }
                    };
                    self.esperar_punc(&TokenKind::Comma, "estrutura_main", "',' no act")?;
                    let value = self.expression("act");
                    self.esperar_punc(&TokenKind::RParen, "estrutura_main", "')' no act")?;
                    Some(Statement::Act { actor, value })
                }
                "every" => {
                    if profundeza >= MAX_EVERY_DEPTH {
                        self.diags.error(
                            "every_muito_profundo",
                            span,
                            format!("aninhamento de 'every' além de {MAX_EVERY_DEPTH} níveis"),
                        );
                        return None;
                    }
                    let period = self.duracao();
                    self
                        .esperar_punc(&TokenKind::LBrace, "estrutura_main", "'{' no every")?;
                    let mut body = Vec::new();
                    loop {
                        if self.eh_punc(TokenKind::RBrace) {
                            self.avancar();
                            break;
                        }
                        match self.peek() {
                            None => {
                                self.diags.error(
                                    "bloco_nao_fechado",
                                    span,
                                    "every sem '}'",
                                );
                                return None;
                            }
                            Some(_) => {
                                if let Some(st) = self.statement(profundeza + 1) {
                                    body.push(st);
                                } else {
                                    self.consumir_ate_virgula_ou_fecha();
                                }
                                self.consumir_virgula("every");
                            }
                        }
                    }
                    Some(Statement::Every { period, body })
                }
                outra => {
                    self.diags.error(
                        "statement_desconhecido",
                        span,
                        format!("statement '{outra}' não existe no main (keep|act|every)"),
                    );
                    None
                }
            },
            t => {
                let found = t.texto();
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
    fn clausulas_cruzadas(&mut self, programa: &Program) {
        let mut formas: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut reviews_vistas: std::collections::BTreeMap<&str, u32> =
            std::collections::BTreeMap::new();

        // 1ª passada: conjunto de formas declaradas (a ligação review→forma
        // é independente da ordem das declarações — EBNF permite intercalar)
        for decl in &programa.decls {
            if let Declaration::Form(f) = decl {
                if !formas.insert(f.name.as_str()) {
                    self.diags.error(
                        "forma_duplicada",
                        f.span,
                        format!("forma '{}' declarada duas vezes — regras não são mescladas", f.name),
                    );
                }
            }
        }

        // 2ª passada: reviews órfãs/duplicadas
        for decl in &programa.decls {
            if let Declaration::Review(r) = decl {
                if !formas.contains(r.form.as_str()) {
                    self.diags.error(
                        "review_orfa",
                        r.span,
                        format!(
                            "review para forma inexistente: '{}' — erro de compilação (FORMAL §3)",
                            r.form
                        ),
                    );
                }
                let vistas = reviews_vistas.entry(r.form.as_str()).or_insert(0);
                *vistas += 1;
                if *vistas > 1 {
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
        if let Some(main) = &programa.main {
            fn passe(stmts: &[Statement], formas: &std::collections::BTreeSet<&str>, diags: &mut Diagnostics, main_span: Span) {
                for st in stmts {
                    match st {
                        Statement::Keep(nome) => {
                            if !formas.contains(nome.as_str()) {
                                diags.error(
                                    "keep_forma_inexistente",
                                    main_span,
                                    format!(
                                        "keep('{nome}') não aponta para forma declarada — cláusula de erro"
                                    ),
                                );
                            }
                        }
                        Statement::Every { body, .. } => passe(body, formas, diags, main_span),
                        Statement::Act { .. } => {}
                    }
                }
            }
            passe(&main.statements, &formas, &mut self.diags, main.span);
        }
    }
}
