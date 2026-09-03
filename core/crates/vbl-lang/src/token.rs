//! Unidades léxicas (FORMAL §2).
//!
//! O lexer reconhece os tokens pela categoria; palavras-chave são
//! diferenciadas pelo parser (texto do identificador) — exceto `°C` e `%`,
//! que têm categoria própria por não serem identificadores.

use crate::diag::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// `[a-zA-Z_][a-zA-Z0-9_]*` — inclui palavras-chave (o parser decide).
    Ident(String),
    /// `[0-9]+`
    Int(i128),
    /// `[0-9]+ '.' [0-9]+`
    Decimal(f64),
    /// string decodificada (escapes `\" \\ \n \t` resolvidos)
    Str(String),
    /// `°C` — unidade física de temperatura
    DegC,
    /// `%` — porcentagem
    Percent,
    Colon,  // :
    Comma,  // ,
    Semi,   // ;
    LBrace, // {
    RBrace, // }
    LParen, // (
    RParen, // )
    Arrow,  // ->
    Lt,     // <
    Gt,     // >
    Le,     // <=
    Ge,     // >=
    EqEq,   // ==
    NotEq,  // !=
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    /// Texto de exibição do token em mensagens de erro.
    pub fn text(&self) -> String {
        match &self.kind {
            TokenKind::Ident(s) => s.clone(),
            TokenKind::Int(n) => n.to_string(),
            TokenKind::Decimal(x) => x.to_string(),
            TokenKind::Str(s) => format!("\"{s}\""),
            TokenKind::DegC => "°C".into(),
            TokenKind::Percent => "%".into(),
            TokenKind::Colon => ":".into(),
            TokenKind::Comma => ",".into(),
            TokenKind::Semi => ";".into(),
            TokenKind::LBrace => "{".into(),
            TokenKind::RBrace => "}".into(),
            TokenKind::LParen => "(".into(),
            TokenKind::RParen => ")".into(),
            TokenKind::Arrow => "->".into(),
            TokenKind::Lt => "<".into(),
            TokenKind::Gt => ">".into(),
            TokenKind::Le => "<=".into(),
            TokenKind::Ge => ">=".into(),
            TokenKind::EqEq => "==".into(),
            TokenKind::NotEq => "!=".into(),
        }
    }
}

/// Palavras-chave da linguagem (FORMAL §2) — o parser as reconhece pelo texto.
pub mod kw {
    pub const CONJUGATIONS: [&str; 3] = ["event", "equilibrium", "nonequilibrium"];
    pub const DECLARATIONS: [&str; 2] = ["review", "main"];
    pub const CONTROLE: [&str; 4] = ["when", "keep", "every", "review"];
    pub const ACTIONS: [&str; 6] = [
        "dissolve",
        "subvert",
        "reclassify_as_equilibrium",
        "reclassify_as_nonequilibrium",
        "notify_shutdown",
        "act",
    ];
    pub const ATTRIBUTES: [&str; 8] = [
        "value",
        "horizon",
        "source_path",
        "maintenance_deadline",
        "exchange_mode",
        "cost_bytes",
        "currency",
        "classification",
    ];
    /// Unidades de tempo e físicas são identificadores reservados no contexto
    /// de duração/threshold (o parser valida pela tabela).
    pub const TIME_UNITS: [&str; 4] = ["s", "ms", "us", "ns"];
    pub const W: &str = "W";
}
