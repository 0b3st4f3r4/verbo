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
    Nulo,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn obj(campos: impl IntoIterator<Item = (&'static str, Json)>) -> Json {
        Json::Obj(campos.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
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

    pub fn serializar(&self) -> String {
        let mut out = String::new();
        self.escrever(&mut out);
        out
    }

    fn escrever(&self, out: &mut String) {
        match self {
            Json::Nulo => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.fract() == 0.0 && n.is_finite() && n.abs() < 9.0e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else if n.is_finite() {
                    out.push_str(&format!("{n}"));
                } else {
                    out.push_str("null"); // NaN/∞ não têm representação JSON
                }
            }
            Json::Str(s) => escrever_string(s, out),
            Json::Arr(itens) => {
                out.push('[');
                for (i, item) in itens.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.escrever(out);
                }
                out.push(']');
            }
            Json::Obj(campos) => {
                out.push('{');
                for (i, (k, v)) in campos.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    escrever_string(k, out);
                    out.push(':');
                    v.escrever(out);
                }
                out.push('}');
            }
        }
    }
}

fn escrever_string(s: &str, out: &mut String) {
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

// ----------------------------------------------------------------------
// Parser mínimo (Etapa 4 — verificação externa do JSONL/binário)
// ----------------------------------------------------------------------

impl Json {
    /// Analisa um documento JSON (apenas o necessário para a auditoria do
    /// Caderno: objetos, arrays, strings com escapes, números, bool, null).
    /// Zero dependências; determinístico.
    pub fn analisar(texto: &str) -> Option<Json> {
        let bytes = texto.as_bytes();
        let mut pos = 0usize;
        let valor = analisar_valor(bytes, &mut pos)?;
        pular_espaco(bytes, &mut pos);
        if pos != bytes.len() {
            return None; // sobra de conteúdo — documento inválido
        }
        Some(valor)
    }
}

fn pular_espaco(b: &[u8], pos: &mut usize) {
    while *pos < b.len() && matches!(b[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

fn analisar_valor(b: &[u8], pos: &mut usize) -> Option<Json> {
    pular_espaco(b, pos);
    match *b.get(*pos)? {
        b'{' => analisar_objeto(b, pos),
        b'[' => analisar_array(b, pos),
        b'"' => analisar_string(b, pos).map(Json::Str),
        b't' => analisar_literal(b, pos, "true", Json::Bool(true)),
        b'f' => analisar_literal(b, pos, "false", Json::Bool(false)),
        b'n' => analisar_literal(b, pos, "null", Json::Nulo),
        _ => analisar_numero(b, pos),
    }
}

fn analisar_literal(b: &[u8], pos: &mut usize, literal: &str, valor: Json) -> Option<Json> {
    if b.get(*pos..*pos + literal.len())? == literal.as_bytes() {
        *pos += literal.len();
        Some(valor)
    } else {
        None
    }
}

fn analisar_numero(b: &[u8], pos: &mut usize) -> Option<Json> {
    let inicio = *pos;
    if b.get(*pos) == Some(&b'-') {
        *pos += 1;
    }
    while matches!(b.get(*pos), Some(c) if c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
        *pos += 1;
    }
    let texto = std::str::from_utf8(b.get(inicio..*pos)?).ok()?;
    texto.parse::<f64>().ok().map(Json::Num)
}

fn analisar_string(b: &[u8], pos: &mut usize) -> Option<String> {
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
                let inicio = *pos;
                while *pos < b.len() && b[*pos] != b'"' && b[*pos] != b'\\' {
                    *pos += 1;
                }
                out.push_str(std::str::from_utf8(b.get(inicio..*pos)?).ok()?);
                let _ = c;
            }
        }
    }
}

fn analisar_objeto(b: &[u8], pos: &mut usize) -> Option<Json> {
    *pos += 1; // '{'
    let mut campos = std::collections::BTreeMap::new();
    pular_espaco(b, pos);
    if b.get(*pos) == Some(&b'}') {
        *pos += 1;
        return Some(Json::Obj(campos));
    }
    loop {
        pular_espaco(b, pos);
        let chave = analisar_string(b, pos)?;
        pular_espaco(b, pos);
        if b.get(*pos) != Some(&b':') {
            return None;
        }
        *pos += 1;
        let valor = analisar_valor(b, pos)?;
        campos.insert(chave, valor);
        pular_espaco(b, pos);
        match b.get(*pos)? {
            b',' => *pos += 1,
            b'}' => {
                *pos += 1;
                return Some(Json::Obj(campos));
            }
            _ => return None,
        }
    }
}

fn analisar_array(b: &[u8], pos: &mut usize) -> Option<Json> {
    *pos += 1; // '['
    let mut itens = Vec::new();
    pular_espaco(b, pos);
    if b.get(*pos) == Some(&b']') {
        *pos += 1;
        return Some(Json::Arr(itens));
    }
    loop {
        let valor = analisar_valor(b, pos)?;
        itens.push(valor);
        pular_espaco(b, pos);
        match b.get(*pos)? {
            b',' => *pos += 1,
            b']' => {
                *pos += 1;
                return Some(Json::Arr(itens));
            }
            _ => return None,
        }
    }
}
