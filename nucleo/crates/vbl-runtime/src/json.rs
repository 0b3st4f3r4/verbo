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
