//! Serialização `.vl` canônica (FORMAL §4.1): a forma gravada é
//! **reparseável** pelo parser desta mesma crate — criterio testado por
//! roundtrip. Sem vírgula final (a EBNF não a permite); `value` primeiro,
//! `horizon` depois, opcionais aplicáveis na sequência canônica.

use crate::ast::{Conjugation, Duration, ExprKind, Expression, FormDecl, TimeUnit};

/// Formata número para o `.vl` canônico: inteiro sem ponto, decimal puro.
pub fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 && x.is_finite() {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// Serializa uma expressão como literal canônico (string com escapes,
/// número puro ou identificador).
pub fn fmt_expression(expr: &Expression) -> String {
    match &expr.kind {
        ExprKind::Str(s) => fmt_string_literal(s),
        ExprKind::Num(x) => fmt_num(*x),
        ExprKind::Ident(id) => id.clone(),
    }
}

/// Serializa uma string com escapes `\"`, `\\`, `\n`, `\t` (FORMAL §2).
pub fn fmt_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            outro => out.push(outro),
        }
    }
    out.push('"');
    out
}

/// Serializa uma duração canônica (ex.: `60s`, `2.5s`, `500ms`).
pub fn fmt_duration(d: &Duration) -> String {
    // normaliza sub-secondo para a menor unidade integral quando possível
    let seg = d.segundos();
    if seg.fract() == 0.0 && seg.is_finite() && d.unit == TimeUnit::S {
        format!("{}s", fmt_num(seg))
    } else {
        format!("{}{}", fmt_num(d.valor), d.unit.sufixo())
    }
}

/// Serializa a forma em texto `.vl` canônico reparseável (FORMAL §4.1).
pub fn form_to_vl(f: &FormDecl) -> String {
    let mut linhas = vec![format!("{} {} {{", f.conjugation.nome(), f.name)];
    linhas.push(format!("    value: {},", fmt_expression(&f.value)));
    let mut extras: Vec<String> = Vec::new();
    if let Some(sp) = &f.attrs.source_path {
        extras.push(format!("source_path: {}", fmt_string_literal(sp)));
    }
    if f.conjugation == Conjugation::Nonequilibrium {
        if let Some(dl) = &f.attrs.maintenance_deadline {
            extras.push(format!("maintenance_deadline: {}", fmt_duration(dl)));
        }
        let modo = f.attrs.exchange_mode.clone().unwrap_or_else(|| "cooperation".into());
        extras.push(format!("exchange_mode: {}", fmt_string_literal(&modo)));
    }
    if f.conjugation == Conjugation::Equilibrium {
        if let Some(cb) = f.attrs.cost_bytes {
            extras.push(format!("cost_bytes: {cb}"));
        }
    }
    if let Some(cur) = &f.attrs.currency {
        if cur != f.conjugation.currency_padrao() {
            extras.push(format!("currency: {}", fmt_string_literal(cur)));
        }
    }
    if let Some(cl) = &f.attrs.classification {
        extras.push(format!("classification: {}", fmt_string_literal(cl)));
    }
    if extras.is_empty() {
        linhas.push(format!("    horizon: {}", fmt_duration(&f.horizon)));
    } else {
        linhas.push(format!("    horizon: {},", fmt_duration(&f.horizon)));
        for extra in &extras[..extras.len() - 1] {
            linhas.push(format!("    {extra},"));
        }
        linhas.push(format!("    {}", extras[extras.len() - 1]));
    }
    linhas.push("}".into());
    linhas.join("\n") + "\n"
}
