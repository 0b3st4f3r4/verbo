//! JSON mínimo (apenas serialização) para o Caderno — objetos com chaves em
//! ordem de classificação (`BTreeMap`), determinismo total, zero dependências.
//!
//! A exportação JSONL do Caderno é auditada externamente recomputando a
//! cadeia SHA-256 a partir do arquivo; o formato precisa ser estável e
//! documentado — números integrais são gravados sem casa decimal
//! (`200`, não `200.0`).

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn obj(fields: impl IntoIterator<Item = (&'static str, Json)>) -> Json {
        Json::Obj(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    pub fn num(n: f64) -> Json {
        Json::Num(n)
    }

    pub fn boolean(b: bool) -> Json {
        Json::Bool(b)
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    /// Serializa diretamente no buffer (Etapa 5 — caminho quente do Caderno
    /// reutiliza a string; mesmo output de [`Json::serialize`]).
    pub fn serialize_into(&self, out: &mut String) {
        self.write(out);
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => write_number(*n, out),
            Json::Str(s) => write_string(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Obj(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Escapa e escreve uma string JSON diretamente no buffer de saída (sem
/// alocação intermediária). Etapa 5: formatter ÚNICO da composição canônica —
/// o caminho direto do Caderno de produção escreve por aqui para garantir
/// linhas byte a byte idênticas às do serializador geral.
pub(crate) fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Escreve um número no formato canônico do Caderno (integrais sem casa
/// decimal; NaN/∞ como `null`) — mesma regra do braço `Json::Num` de
/// [`Json::write`], extraída para o caminho direto reutilizar.
pub(crate) fn write_number(n: f64, out: &mut String) {
    if n.fract() == 0.0 && n.is_finite() && n.abs() < 9.0e15 {
        let _ = std::fmt::Write::write_fmt(out, format_args!("{}", n as i64));
    } else if n.is_finite() {
        let _ = std::fmt::Write::write_fmt(out, format_args!("{n}"));
    } else {
        out.push_str("null"); // NaN/∞ não têm representação JSON
    }
}

// ----------------------------------------------------------------------
// Parser mínimo (Etapa 4 — verificação externa do JSONL/binário)
// ----------------------------------------------------------------------

impl Json {
    /// Analisa um documento JSON (apenas o necessário para a auditoria do
    /// Caderno: objetos, arrays, strings com escapes, números, bool, null).
    /// Zero dependências; determinístico.
    pub fn parse(text: &str) -> Option<Json> {
        let bytes = text.as_bytes();
        let mut pos = 0usize;
        let value = parse_value(bytes, &mut pos)?;
        skip_space(bytes, &mut pos);
        if pos != bytes.len() {
            return None; // sobra de conteúdo — documento inválido
        }
        Some(value)
    }
}

fn skip_space(b: &[u8], pos: &mut usize) {
    while *pos < b.len() && matches!(b[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

fn parse_value(b: &[u8], pos: &mut usize) -> Option<Json> {
    skip_space(b, pos);
    match *b.get(*pos)? {
        b'{' => parse_object(b, pos),
        b'[' => parse_array(b, pos),
        b'"' => parse_string(b, pos).map(Json::Str),
        b't' => parse_literal(b, pos, "true", Json::Bool(true)),
        b'f' => parse_literal(b, pos, "false", Json::Bool(false)),
        b'n' => parse_literal(b, pos, "null", Json::Null),
        _ => parse_number(b, pos),
    }
}

fn parse_literal(b: &[u8], pos: &mut usize, literal: &str, value: Json) -> Option<Json> {
    if b.get(*pos..*pos + literal.len())? == literal.as_bytes() {
        *pos += literal.len();
        Some(value)
    } else {
        None
    }
}

fn parse_number(b: &[u8], pos: &mut usize) -> Option<Json> {
    let start = *pos;
    if b.get(*pos) == Some(&b'-') {
        *pos += 1;
    }
    while matches!(b.get(*pos), Some(c) if c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        *pos += 1;
    }
    let text = std::str::from_utf8(b.get(start..*pos)?).ok()?;
    text.parse::<f64>().ok().map(Json::Num)
}

fn parse_string(b: &[u8], pos: &mut usize) -> Option<String> {
    if b.get(*pos) != Some(&b'"') {
        return None;
    }
    *pos += 1;
    let mut out = String::new();
    loop {
        match *b.get(*pos)? {
            b'"' => {
                *pos += 1;
                return Some(out);
            }
            b'\\' => {
                *pos += 1;
                match *b.get(*pos)? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'u' => {
                        let hex = std::str::from_utf8(b.get(*pos + 1..*pos + 5)?).ok()?;
                        let cp = u32::from_str_radix(hex, 16).ok()?;
                        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        *pos += 4;
                    }
                    _ => return None,
                }
                *pos += 1;
            }
            c => {
                // continua o fluxo UTF-8 byte a byte (seguro: fonte era &str)
                let start = *pos;
                while *pos < b.len() && b[*pos] != b'"' && b[*pos] != b'\\' {
                    *pos += 1;
                }
                out.push_str(std::str::from_utf8(b.get(start..*pos)?).ok()?);
                let _ = c;
            }
        }
    }
}

fn parse_object(b: &[u8], pos: &mut usize) -> Option<Json> {
    *pos += 1; // '{'
    let mut fields = std::collections::BTreeMap::new();
    skip_space(b, pos);
    if b.get(*pos) == Some(&b'}') {
        *pos += 1;
        return Some(Json::Obj(fields));
    }
    loop {
        skip_space(b, pos);
        let key = parse_string(b, pos)?;
        skip_space(b, pos);
        if b.get(*pos) != Some(&b':') {
            return None;
        }
        *pos += 1;
        let value = parse_value(b, pos)?;
        fields.insert(key, value);
        skip_space(b, pos);
        match b.get(*pos)? {
            b',' => *pos += 1,
            b'}' => {
                *pos += 1;
                return Some(Json::Obj(fields));
            }
            _ => return None,
        }
    }
}

fn parse_array(b: &[u8], pos: &mut usize) -> Option<Json> {
    *pos += 1; // '['
    let mut items = Vec::new();
    skip_space(b, pos);
    if b.get(*pos) == Some(&b']') {
        *pos += 1;
        return Some(Json::Arr(items));
    }
    loop {
        let value = parse_value(b, pos)?;
        items.push(value);
        skip_space(b, pos);
        match b.get(*pos)? {
            b',' => *pos += 1,
            b']' => {
                *pos += 1;
                return Some(Json::Arr(items));
            }
            _ => return None,
        }
    }
}
