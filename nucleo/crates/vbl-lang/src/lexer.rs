//! Lexer — varre o fonte e produz tokens (FORMAL §2), com diagnósticos
//! léxicos estritos: caractere fora da linguagem vira `lexema_invalido`
//! (reancora `tests/vlcheck.py::test_...` — ex.: `=` isolado é rejeitado).
//!
//! Comentários `//` e `/* ... */` são descartados; strings têm escapes
//! `\"`, `\\`, `\n`, `\t` e limite de 256 bytes decodificados (FORMAL §2).

use crate::diag::{Diagnostics, Span};
use crate::token::{Token, TokenKind};

pub const LIMITE_STRING_BYTES: usize = 256;

pub struct Lexer {
    src: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    pub diags: Diagnostics,
}

impl Lexer {
    pub fn new(fonte: &str) -> Self {
        Self {
            src: fonte.chars().collect(),
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
                self.pular_linha();
                continue;
            }
            if *c == '/' && self.peek2() == Some(&'*') {
                self.pular_bloco();
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
                    let kind = self.numero();
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
                    let nome = self.identificador();
                    tokens.push(Token { kind: TokenKind::Ident(nome), span });
                }
                _ => {
                    let outro = *c;
                    self.bump();
                    self.diags.error(
                        "lexema_invalido",
                        span,
                        format!("lexema {outro:?} não existe na linguagem"),
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

    fn consume(&mut self, esperado: char) -> bool {
        if self.peek() == Some(&esperado) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn here(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn pular_linha(&mut self) {
        while let Some(c) = self.peek() {
            if *c == '\n' {
                break;
            }
            self.bump();
        }
    }

    fn pular_bloco(&mut self) {
        let inicio = self.here();
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
        self.diags.error("comentario_nao_fechado", inicio, "comentário de bloco sem '*/'");
    }

    fn identificador(&mut self) -> String {
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

    fn numero(&mut self) -> TokenKind {
        let mut texto = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                texto.push(*c);
                self.bump();
            } else {
                break;
            }
        }
        // decimal: dígito seguido de '.' e pelo menos um dígito (FORMAL §2)
        if self.peek() == Some(&'.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            texto.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    texto.push(*c);
                    self.bump();
                } else {
                    break;
                }
            }
            // f64 cobre os valores da linguagem (thresholds/durações);
            // inteiros além de i128 caem aqui, o que a EBNF não produz.
            return TokenKind::Decimal(texto.parse::<f64>().unwrap_or(f64::INFINITY));
        }
        match texto.parse::<i128>() {
            Ok(n) => TokenKind::Int(n),
            Err(_) => {
                // inteiro gigante: preserva como decimal (com aviso)
                TokenKind::Decimal(texto.parse::<f64>().unwrap_or(f64::INFINITY))
            }
        }
    }

    fn string(&mut self) -> Option<TokenKind> {
        let span = self.here();
        self.bump(); // '"'
        let mut valor = String::new();
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
                    Some('"') => valor.push('"'),
                    Some('\\') => valor.push('\\'),
                    Some('n') => valor.push('\n'),
                    Some('t') => valor.push('\t'),
                    // escapes não canônicos preservam o caractere (contrato
                    // do validador de superfície da Etapa 1)
                    Some(outro) => valor.push(outro),
                    None => {
                        self.diags.error(
                            "string_nao_terminada",
                            span,
                            "string termina em escape '\\'",
                        );
                        return None;
                    }
                },
                Some(c) => valor.push(c),
            }
        }
        let bytes = valor.len();
        if bytes > LIMITE_STRING_BYTES {
            self.diags.error(
                "string_muito_longa",
                span,
                format!("string excede {LIMITE_STRING_BYTES} bytes (tem {bytes})"),
            );
        }
        Some(TokenKind::Str(valor))
    }
}

/// Tokeniza o fonte, devolvendo tokens e diagnósticos.
pub fn tokenize(fonte: &str) -> (Vec<Token>, Diagnostics) {
    Lexer::new(fonte).tokenize()
}
