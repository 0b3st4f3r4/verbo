//! Roundtrip do schema v1 — o critério de aceite da Etapa 3 "protocolo FXP
//! serializa/desserializa sem perda" (AGENTS §2.2), mais a rejeição total de
//! mensagens malformadas (decodificador nunca devolve mensagem parcial).

use vbl_fxp::schema::{
    decode, encode_to_vec, peek_frame_len, AckAct, Body, DeviceDesc, SchemaError, Message,
    WireValue, HEADER_LEN, MAGIC, MAX_NAME, MAX_PAYLOAD, MAX_STRING, VERSION,
};

/// Corpus representativo: toda opcode × corpo × valor-limite.
fn corpus() -> Vec<Message> {
    vec![
        Message::read("cpu_temp", 1, true),
        Message::read("human_attention", 0, false), // alias (FORMAL §6)
        Message {
            opcode: vbl_fxp::schema::op::ACT,
            flags: 0,
            seq: u32::MAX,
            name: "C".repeat(MAX_NAME),
            body: Body::Act { value: WireValue::Num(0.0) }, // 0.0 é leitura/comando válido
        },
        Message::act(
            "LedIndicador",
            WireValue::Str("verde".into()),
            42,
            true,
        ),
        Message::act("Ventoinha", WireValue::Ident("verde".into()), 43, true),
        Message::read_ok(-273.15, "cpu_temp", false, 7),
        Message::read_ok(f64::MIN_POSITIVE, "attention", true, 8),
        Message::read_err(0, 9),
        Message::read_err(3, 10),
        Message::act_ack(AckAct::Delivered, false, 11),
        Message::act_ack(AckAct::Rejected { limit: 2, limit_value: 200.0 }, false, 12),
        Message::act_ack(AckAct::MissingActor, false, 13),
        Message::act_ack(AckAct::Unavailable, false, 14),
        Message::act_ack(AckAct::FallbackExecuted { alternativo: "VentoinhaReserva".into() }, true, 15),
        Message::act_ack(AckAct::FallbackExhausted, false, 16),
        Message::act_ack(AckAct::InvalidValue { reason: "cor desconhecida: 'roxo'".into() }, false, 17),
        Message::heartbeat("Ventoinha", 18),
        Message::heartbeat_ack(true, 19),
        Message::heartbeat_ack(false, 20),
        Message::hello(vec![], 21),
        Message::hello(
            vec![
                DeviceDesc::Sensor {
                    name: "cpu_temp".into(),
                    min: Some(0.0), // mínimo legítimo 0 (não é "não declarado")
                    max: Some(120.0),
                    quantity: "temperatura".into(),
                    unit: "°C".into(),
                    precision_pct: 2.0,
                },
                DeviceDesc::Sensor {
                    name: "attention".into(),
                    min: None,
                    max: None,
                    quantity: "atenção".into(),
                    unit: "%".into(),
                    precision_pct: 0.0,
                },
                DeviceDesc::Actor {
                    name: "CpuPowerCap".into(),
                    min: Some(10.0),
                    max: Some(250.0),
                    safety: Some(200.0),
                },
            ],
            22,
        ),
        Message::bye(23),
    ]
}

#[test]
fn roundtrip_is_bit_for_bit_identity_for_corpus() {
    for msg in corpus() {
        let bytes = encode_to_vec(&msg).unwrap_or_else(|e| panic!("encode falhou: {e}"));
        let (volta, consumed) = decode(&bytes).unwrap_or_else(|e| panic!("decode falhou: {e}"));
        assert_eq!(consumed, bytes.len(), "frame deve ser consumido por inteiro");
        assert_eq!(volta, msg, "roundtrip alterou a mensagem");
    }
}

#[test]
fn f64_preserves_all_64_bits() {
    // Amostragem de padrões de bits finitos (inclui negativos, subnormais,
    // épsilon, extremos) — nenhum pode perder um bit no roundtrip.
    let defaults = [
        0u64, 1, 0x8000_0000_0000_0001, // -menor subnormal
        0x0000_0000_0000_0001, // menor subnormal
        0x7FEF_FFFF_FFFF_FFFF, // maior finito
        0xFFEF_FFFF_FFFF_FFFF, // menor finito (negativo)
        0x3FF0_0000_0000_0001, // 1 + épsilon/2
        0x4059_0000_0000_0000, // 100.0
        0xC07F_4000_0000_0000, // -500.0
        0x408F_4000_0000_0000, // 1000.75
        u64::from_le_bytes(86.5f64.to_le_bytes()), // limite térmico da FORMAL
    ];
    for bits in defaults {
        let v = f64::from_bits(bits);
        assert!(v.is_finite(), "corpus de teste deve ser finito");
        let msg = Message::read_ok(v, "cpu_power", false, 1);
        let bytes = encode_to_vec(&msg).unwrap();
        let (back, _) = decode(&bytes).unwrap();
        let Body::ReadOk { value, .. } = back.body else {
            panic!("corpo errado");
        };
        assert_eq!(value.to_bits(), bits, "bit {bits:#018x} alterado no roundtrip");
    }
}

#[test]
fn utf8_multibyte_exact_remainder() {
    let names = ["temperatura_çãã", "cpu_temp_🔥", "grandeza_αβγ_°C_W_%"];
    for (i, s) in names.iter().enumerate() {
        let msg = Message::act(
            "LedIndicador",
            WireValue::Str((*s).into()),
            i as u32,
            true,
        );
        let bytes = encode_to_vec(&msg).unwrap();
        let (back, _) = decode(&bytes).unwrap();
        let Body::Act { value } = back.body else { panic!() };
        assert_eq!(value, WireValue::Str((*s).to_string()));
    }
}

#[test]
fn string_and_name_at_max_limits() {
    let msg = Message::act("A".repeat(MAX_NAME).as_str(), WireValue::Str("x".repeat(MAX_STRING)), 1, true);
    let bytes = encode_to_vec(&msg).unwrap();
    assert!(bytes.len() <= 4 + MAX_PAYLOAD);
    let (back, _) = decode(&bytes).unwrap();
    assert_eq!(back.name.len(), MAX_NAME);
}

#[test]
fn rejections_of_encode() {
    // NaN/inf são falha de I/O, nunca valor no fio (FORMAL §4.7).
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            encode_to_vec(&Message::read_ok(v, "s", false, 1)),
            Err(SchemaError::NonFiniteValue)
        );
    }
    // Nome acima de 255 bytes.
    assert_eq!(
        encode_to_vec(&Message::read(&"n".repeat(MAX_NAME + 1), 1, true)),
        Err(SchemaError::StringTooLong)
    );
    // String acima de 1024 bytes.
    assert_eq!(
        encode_to_vec(&Message::act("a", WireValue::Str("s".repeat(MAX_STRING + 1)), 1, true)),
        Err(SchemaError::StringTooLong)
    );
    // Flags reservadas devem ser 0 no encode.
    let mut msg = Message::read("s", 1, true);
    msg.flags |= 0b0001_0000;
    assert_eq!(encode_to_vec(&msg), Err(SchemaError::ReservedFlag));
    // Payload acima de 8192: HELLO com muitos descritores transborda o guard.
    let flood: Vec<DeviceDesc> = (0..500)
        .map(|i| DeviceDesc::Sensor {
            name: format!("s{i}"),
            min: None,
            max: None,
            quantity: "g".into(),
            unit: "u".into(),
            precision_pct: 1.0,
        })
        .collect();
    assert!(matches!(
        encode_to_vec(&Message::hello(flood, 1)),
        Err(SchemaError::FrameExceeded { .. })
    ));
}

#[test]
fn decode_rejections_never_return_partial_message() {
    let valid = encode_to_vec(&Message::read_ok(42.0, "cpu_temp", true, 5)).unwrap();

    // Truncamento em cada byte do frame.
    for n in 0..valid.len() {
        let trunc = &valid[..n];
        assert!(
            decode(trunc).is_err(),
            "truncado em {n} bytes não pode decodificar"
        );
    }

    // Magic inválido.
    let mut bad = valid.clone();
    bad[4] = b'X';
    assert_eq!(decode(&bad).unwrap_err(), SchemaError::InvalidMagic);

    // Versão desconhecida.
    let mut bad = valid.clone();
    bad[3 + 4] = 2;
    assert_eq!(decode(&bad).unwrap_err(), SchemaError::UnknownVersion { received: 2 });

    // Opcode desconhecido.
    let mut bad = valid.clone();
    bad[4 + 4] = 0x7F;
    assert_eq!(decode(&bad).unwrap_err(), SchemaError::UnknownOpcode { received: 0x7F });

    // Header direto (sem o prefixo de comprimento): magic vira "length" absurdo.
    let header = &valid[4..];
    assert!(matches!(
        decode(header),
        Err(SchemaError::FrameExceeded { .. })
    ));

    // Payload declarado menor que o header fixo.
    let mut short = Vec::new();
    short.extend_from_slice(&8u32.to_le_bytes());
    short.extend_from_slice(&[0u8; 8]);
    assert_eq!(decode(&short).unwrap_err(), SchemaError::PayloadTooShort { length: 8 });

    // NaN no fio é rejeitado no decode (READ_OK: f64 logo após header + nome vazio).
    let msg = Message::read_ok(1.0, "s", false, 1);
    let mut bytes = encode_to_vec(&msg).unwrap();
    let off = 4 + HEADER_LEN;
    bytes[off..off + 8].copy_from_slice(&f64::NAN.to_bits().to_le_bytes());
    assert_eq!(decode(&bytes).unwrap_err(), SchemaError::NonFiniteValue);

    // Bytes sobrando após o corpo (length +1, byte extra no fim).
    let mut extra = encode_to_vec(&Message::read_err(1, 1)).unwrap();
    let len = u32::from_le_bytes([extra[0], extra[1], extra[2], extra[3]]);
    let new = len + 1;
    extra[0..4].copy_from_slice(&new.to_le_bytes());
    extra.push(0);
    assert_eq!(decode(&extra).unwrap_err(), SchemaError::PayloadTooLong);
}

#[test]
fn peek_frame_len_and_framing_on_stream() {
    let bytes = encode_to_vec(&Message::read("cpu_temp", 1, true)).unwrap();
    // Metade chegou: peek devolve o total do frame; decode ainda incompleto.
    let total = peek_frame_len(&bytes).unwrap();
    assert_eq!(total, bytes.len());
    assert!(matches!(decode(&bytes[..total / 2]), Err(SchemaError::MissingField)));
    // Prefixo completo, payload pela metade.
    assert!(decode(&bytes[..4]).is_err());
    // Frame acima do máximo é rejeitado no peek.
    let mut bad = Vec::new();
    bad.extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
    assert!(matches!(
        peek_frame_len(&bad),
        Err(SchemaError::FrameExceeded { .. })
    ));
}

#[test]
fn header_v1_is_little_endian_per_doc() {
    let msg = Message::read("cpu_temp", 0x0102_0304, true);
    let b = encode_to_vec(&msg).unwrap();
    assert_eq!(&b[4..7], &MAGIC);
    assert_eq!(b[7], VERSION);
    assert_eq!(b[8], vbl_fxp::schema::op::READ);
    assert_eq!(b[9], vbl_fxp::schema::flag::ACK);
    assert_eq!(b[10], 0, "reservado");
    assert_eq!(b[11], 8, "len(cpu_temp)");
    assert_eq!(&b[12..16], &[0x04, 0x03, 0x02, 0x01], "seq LE");
    let length = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    assert_eq!(length, HEADER_LEN + 8);
    assert_eq!(b.len(), 4 + length);
}
