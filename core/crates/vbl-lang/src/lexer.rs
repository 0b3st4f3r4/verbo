//! Lexer — varre o fonte e produz tokens (FORMAL §2), com diagnósticos
//! léxicos estritos: caractere fora da linguagem vira `lexema_invalido`
//! (reancora `tests/vlcheck.py::test_...` — ex.: `=` isolado é rejeitado).
//!
//! Comentários `//` e `/* ... */` são descartados; strings têm escapes
//! `\"`, `\\`, `\n`, `\t` e limite de 256 bytes decodificados (FORMAL §2).

use crate::diag::{Diagnostics, Span};
use crate::token::{Token, TokenKind};

pub const LIMIT_STRING_BYTES: usize = 256;

pub struct Lexer {
    src: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    pub diags: Diagnostics,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            src: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            diags: Diagnostics::new(),
        }
    }

    pub fn tokenize(mut self) -> (Vec<Token>, Diagnostics) {
        let mut tokens = Vec::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.bump();
                continue;
            }
            if *c == '/' && self.peek2() == Some(&'/') {
                self.skip_line();
                continue;
            }
            if *c == '/' && self.peek2() == Some(&'*') {
                self.skip_block();
                continue;
            }
            let span = self.here();
            match c {
                '"' => {
                    if let Some(kind) = self.string() {
                        tokens.push(Token { kind, span });
                    }
                }
                '0'..='9' => {
                    let kind = self.number();
                    tokens.push(Token { kind, span });
                }
                '°' => {
                    self.bump();
                    if self.peek() == Some(&'C') {
                        self.bump();
                        tokens.push(Token { kind: TokenKind::DegC, span });
                    } else {
                        self.diags.error(
                            "lexema_invalido",
                            span,
                            "lexema '°' sem 'C' — a unidade canônica é '°C'",
                        );
                    }
                }
                '%' => {
                    self.bump();
                    tokens.push(Token { kind: TokenKind::Percent, span });
                }
                ':' => self.punc(&mut tokens, span, TokenKind::Colon),
                ',' => self.punc(&mut tokens, span, TokenKind::Comma),
                ';' => self.punc(&mut tokens, span, TokenKind::Semi),
                '{' => self.punc(&mut tokens, span, TokenKind::LBrace),
                '}' => self.punc(&mut tokens, span, TokenKind::RBrace),
                '(' => self.punc(&mut tokens, span, TokenKind::LParen),
                ')' => self.punc(&mut tokens, span, TokenKind::RParen),
                '<' => {
                    self.bump();
                    let kind = if self.consume('=') { TokenKind::Le } else { TokenKind::Lt };
                    tokens.push(Token { kind, span });
                }
                '>' => {
                    self.bump();
                    let kind = if self.consume('=') { TokenKind::Ge } else { TokenKind::Gt };
                    tokens.push(Token { kind, span });
                }
                '=' => {
                    self.bump();
                    if self.consume('=') {
                        tokens.push(Token { kind: TokenKind::EqEq, span });
                    } else {
                        self.diags.error(
                            "lexema_invalido",
                            span,
                            "lexema '=' não existe na linguagem (comparação é '==')",
                        );
                    }
                }
                '!' => {
                    self.bump();
                    if self.consume('=') {
                        tokens.push(Token { kind: TokenKind::NotEq, span });
                    } else {
                        self.diags
                            .error("lexema_invalido", span, "lexema '!' não existe na linguagem");
                    }
                }
                '-' => {
                    self.bump();
                    if self.consume('>') {
                        tokens.push(Token { kind: TokenKind::Arrow, span });
                    } else {
                        self.diags.error(
                            "lexema_invalido",
                            span,
                            "lexema '-' não existe na linguagem (a seta é '->')",
                        );
                    }
                }
                c if c.is_ascii_alphabetic() || *c == '_' => {
                    let name = self.identifier();
                    tokens.push(Token { kind: TokenKind::Ident(name), span });
                }
                _ => {
                    let other = *c;
                    self.bump();
                    self.diags.error(
                        "lexema_invalido",
                        span,
                        format!("lexema {other:?} não existe na linguagem"),
                    );
                }
            }
        }
        self.diags.sort();
        (tokens, self.diags)
    }

    fn punc(&mut self, tokens: &mut Vec<Token>, span: Span, kind: TokenKind) {
        self.bump();
        tokens.push(Token { kind, span });
    }

    // ------------------------------------------------------------------
    // Primitivas
    // ------------------------------------------------------------------
    fn peek(&self) -> Option<&char> {
        self.src.get(self.pos)
    }

    fn peek2(&self) -> Option<&char> {
        self.src.get(self.pos + 1)
    }

    fn bump(&mut self) -> Option<char> {
        let c = *self.src.get(self.pos)?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(&expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn here(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            if *c == '\n' {
                break;
            }
            self.bump();
        }
    }

    fn skip_block(&mut self) {
        let start = self.here();
        self.bump(); // '/'
        self.bump(); // '*'
        while self.pos < self.src.len() {
            if *self.peek().unwrap() == '*' && self.peek2() == Some(&'/') {
                self.bump();
                self.bump();
                return;
            }
            self.bump();
        }
        self.diags.error("comentario_nao_fechado", start, "comentário de bloco sem '*/'");
    }

    fn identifier(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || *c == '_' {
                s.push(*c);
                self.bump();
            } else {
                break;
            }
        }
        s
    }

    fn number(&mut self) -> TokenKind {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                text.push(*c);
                self.bump();
            } else {
                break;
            }
        }
        // decimal: dígito seguido de '.' e pelo menos um dígito (FORMAL §2)
        if self.peek() == Some(&'.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            text.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    text.push(*c);
                    self.bump();
                } else {
                    break;
                }
            }
            // f64 cobre os valores da linguagem (thresholds/durações);
            // inteiros além de i128 caem aqui, o que a EBNF não produz.
            return TokenKind::Decimal(text.parse::<f64>().unwrap_or(f64::INFINITY));
        }
        match text.parse::<i128>() {
            Ok(n) => TokenKind::Int(n),
            Err(_) => {
                // inteiro gigante: preserva como decimal (com aviso)
                TokenKind::Decimal(text.parse::<f64>().unwrap_or(f64::INFINITY))
            }
        }
    }

    fn string(&mut self) -> Option<TokenKind> {
        let span = self.here();
        self.bump(); // '"'
        let mut value = String::new();
        loop {
            match self.bump() {
                None => {
                    self.diags
                        .error("string_nao_terminada", span, "string sem aspas de fechamento");
                    return None;
                }
                Some('"') => break,
                Some('\n') => {
                    self.diags.error(
                        "string_nao_terminada",
                        span,
                        "quebra de linha dentro de string (strings são de uma linha)",
                    );
                    return None;
                }
                Some('\\') => match self.bump() {
                    Some('"') => value.push('"'),
                    Some('\\') => value.push('\\'),
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    // escapes não canônicos preservam o caractere (contrato
                    // do validador de superfície da Etapa 1)
                    Some(other) => value.push(other),
                    None => {
                        self.diags.error(
                            "string_nao_terminada",
                            span,
                            "string termina em escape '\\'",
                        );
                        return None;
                    }
                },
                Some(c) => value.push(c),
            }
        }
        let bytes = value.len();
        if bytes > LIMIT_STRING_BYTES {
            self.diags.error(
                "string_muito_longa",
                span,
                format!("string excede {LIMIT_STRING_BYTES} bytes (tem {bytes})"),
            );
        }
        Some(TokenKind::Str(value))
    }
}

/// Tokeniza o fonte, devolvendo tokens e diagnósticos.
pub fn tokenize(source: &str) -> (Vec<Token>, Diagnostics) {
    Lexer::new(source).tokenize()
}
