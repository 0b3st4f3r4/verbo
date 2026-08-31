//! Serialização `.vl` canônica (FORMAL §4.1): a forma persistida é
//! reparseável e re-serializa para o mesmo texto (ponto fixo).

use vbl_lang::{canon::form_to_vl, parse, Conjugation, Duration, ExprKind, Expression, FormAttrs, FormDecl, PhysicalUnit, TimeUnit};

fn forma_exemplo(conjugation: Conjugation) -> FormDecl {
    FormDecl {
        conjugation,
        name: "Exemplo".into(),
        value: Expression { kind: ExprKind::Str("conteúdo \"com\" escapes".into()), span: Default::default() },
        horizon: Duration { valor: 60.0, unit: TimeUnit::S, span: Default::default() },
        attrs: FormAttrs {
            source_path: Some("attention".into()),
            maintenance_deadline: Some(Duration { valor: 3.0, unit: TimeUnit::S, span: Default::default() }),
            exchange_mode: Some("cooperation".into()),
            cost_bytes: None,
            currency: None,
            classification: None,
        },
        span: Default::default(),
    }
}

fn roundtrip(f: &FormDecl) -> FormDecl {
    let texto = form_to_vl(f);
    let (programa, diags) = parse(&texto);
    assert!(!diags.has_errors(), "`.vl` canônico não reparseou:\n{texto}\n{diags}");
    let reparseada = programa.forms().next().unwrap().clone();
    assert_eq!(form_to_vl(&reparseada), texto, "serialização não é ponto fixo");
    reparseada
}

#[test]
fn canonequilibrium_sem_opcionais_e_minimo() {
    let f = FormDecl {
        conjugation: Conjugation::Event,
        name: "Piscada".into(),
        value: Expression { kind: ExprKind::Str("impulso_curto".into()), span: Default::default() },
        horizon: Duration { valor: 2.0, unit: TimeUnit::S, span: Default::default() },
        attrs: FormAttrs::default(),
        span: Default::default(),
    };
    let texto = form_to_vl(&f);
    assert_eq!(
        texto,
        "event Piscada {\n    value: \"impulso_curto\",\n    horizon: 2s\n}\n"
    );
    roundtrip(&f);
}

#[test]
fn canon_nonequilibrium_com_opcionais_e_horizon_absoluto_preservado() {
    let mut f = forma_exemplo(Conjugation::Nonequilibrium);
    f.horizon = Duration { valor: 2.5, unit: TimeUnit::S, span: Default::default() };
    let r = roundtrip(&f);
    assert_eq!(r.attrs.maintenance_deadline.unwrap().segundos(), 3.0);
    assert_eq!(r.attrs.source_path.as_deref(), Some("attention"));
    assert_eq!(r.horizon.segundos(), 2.5);
}

#[test]
fn canon_equilibrium_cost_bytes_e_currency_nao_padrao() {
    let mut f = forma_exemplo(Conjugation::Equilibrium);
    f.attrs.source_path = None;
    f.attrs.maintenance_deadline = None;
    f.attrs.exchange_mode = None;
    f.attrs.cost_bytes = Some(4096);
    f.attrs.currency = Some("DiskBytesCustom".into());
    let texto = form_to_vl(&f);
    assert!(texto.contains("cost_bytes: 4096"), "{texto}");
    assert!(texto.contains("currency: \"DiskBytesCustom\""), "{texto}");
    roundtrip(&f);
}

#[test]
fn canon_currency_padrao_nao_e_gravada() {
    let mut f = forma_exemplo(Conjugation::Nonequilibrium);
    f.attrs.currency = Some("PowerWatts".into()); // padrão da conjugação
    let texto = form_to_vl(&f);
    assert!(!texto.contains("currency"), "{texto}");
}

#[test]
fn canon_threshold_com_unidades_sobrevive_ao_roundtrip() {
    let texto = "\
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
    let (p, d) = vbl_lang::parse(texto);
    assert!(!d.has_errors(), "{d}");
    let regra = &p.reviews().next().unwrap().rules[0];
    assert_eq!(regra.threshold.unit, Some(PhysicalUnit::DegC));
    assert_eq!(regra.threshold.valor, 85.0);
}
