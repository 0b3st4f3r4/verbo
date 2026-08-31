//! Serialização `.vl` canônica (FORMAL §4.1): a forma persistida é
//! reparseável e re-serializa para o mesmo texto (ponto fixo).

use vbl_lang::{canon::form_to_vl, parse, Conjugation, Duration, ExprKind, Expression, FormAttrs, FormDecl, PhysicalUnit, TimeUnit};

fn example_form(conjugation: Conjugation) -> FormDecl {
    FormDecl {
        conjugation,
        name: "Exemplo".into(),
        value: Expression { kind: ExprKind::Str("conteúdo \"com\" escapes".into()), span: Default::default() },
        horizon: Duration { value: 60.0, unit: TimeUnit::S, span: Default::default() },
        attrs: FormAttrs {
            source_path: Some("attention".into()),
            maintenance_deadline: Some(Duration { value: 3.0, unit: TimeUnit::S, span: Default::default() }),
            exchange_mode: Some("cooperation".into()),
            cost_bytes: None,
            currency: None,
            classification: None,
        },
        span: Default::default(),
    }
}

fn roundtrip(f: &FormDecl) -> FormDecl {
    let text = form_to_vl(f);
    let (program, diags) = parse(&text);
    assert!(!diags.has_errors(), "`.vl` canônico não reparseou:\n{text}\n{diags}");
    let reparseada = program.forms().next().unwrap().clone();
    assert_eq!(form_to_vl(&reparseada), text, "serialização não é ponto fixo");
    reparseada
}

#[test]
fn canonical_equilibrium_without_optionals_is_minimal() {
    let f = FormDecl {
        conjugation: Conjugation::Event,
        name: "Piscada".into(),
        value: Expression { kind: ExprKind::Str("impulso_curto".into()), span: Default::default() },
        horizon: Duration { value: 2.0, unit: TimeUnit::S, span: Default::default() },
        attrs: FormAttrs::default(),
        span: Default::default(),
    };
    let text = form_to_vl(&f);
    assert_eq!(
        text,
        "event Piscada {\n    value: \"impulso_curto\",\n    horizon: 2s\n}\n"
    );
    roundtrip(&f);
}

#[test]
fn canonical_nonequilibrium_with_optionals_and_absolute_horizon_preserved() {
    let mut f = example_form(Conjugation::Nonequilibrium);
    f.horizon = Duration { value: 2.5, unit: TimeUnit::S, span: Default::default() };
    let r = roundtrip(&f);
    assert_eq!(r.attrs.maintenance_deadline.unwrap().seconds(), 3.0);
    assert_eq!(r.attrs.source_path.as_deref(), Some("attention"));
    assert_eq!(r.horizon.seconds(), 2.5);
}

#[test]
fn canon_equilibrium_cost_bytes_and_currency_non_default() {
    let mut f = example_form(Conjugation::Equilibrium);
    f.attrs.source_path = None;
    f.attrs.maintenance_deadline = None;
    f.attrs.exchange_mode = None;
    f.attrs.cost_bytes = Some(4096);
    f.attrs.currency = Some("DiskBytesCustom".into());
    let text = form_to_vl(&f);
    assert!(text.contains("cost_bytes: 4096"), "{text}");
    assert!(text.contains("currency: \"DiskBytesCustom\""), "{text}");
    roundtrip(&f);
}

#[test]
fn canon_currency_default_not_written() {
    let mut f = example_form(Conjugation::Nonequilibrium);
    f.attrs.currency = Some("PowerWatts".into()); // padrão da conjugação
    let text = form_to_vl(&f);
    assert!(!text.contains("currency"), "{text}");
}

#[test]
fn canonical_threshold_with_units_survives_roundtrip() {
    let text = "\
nonequilibrium T {
    value: \"lucro\",
    horizon: 7s,
    source_path: \"cpu_temp\",
    maintenance_deadline: 2s,
    exchange_mode: \"extraction\"
}

review T {
    when cpu_temp > 85°C -> subvert,
                            act(CpuPowerCap, 50)
}";
    let (p, d) = vbl_lang::parse(text);
    assert!(!d.has_errors(), "{d}");
    let rule = &p.reviews().next().unwrap().rules[0];
    assert_eq!(rule.threshold.unit, Some(PhysicalUnit::DegC));
    assert_eq!(rule.threshold.value, 85.0);
}
