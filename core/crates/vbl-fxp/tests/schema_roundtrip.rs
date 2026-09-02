//! Roundtrip do schema v1 — o critério de aceite da Etapa 3 "protocolo FXP
//! serializa/desserializa sem perda" (AGENTS §2.2), mais a rejeição total de
//! mensagens malformadas (decodificador nunca devolve mensagem parcial).

use vbl_fxp::schema::{
    caps, decode, encode_to_vec, flag, peek_frame_len, reason, AckAct, BatchResult, Body,
    DeviceDesc, Message, SchemaError, WireValue, AUTH_SCHEME_PSK_HMAC_SHA256, HEADER_LEN, MAGIC,
    MAX_BATCH, MAX_NAME, MAX_PAYLOAD, MAX_STRING, VERSION,
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
            timestamp_us: None,
            body: Body::Act { value: WireValue::Num(0.0) }, // 0.0 é leitura/comando válido
        },
        Message::act(
            "StatusLed",
            WireValue::Str("green".into()),
            42,
            true,
        ),
        Message::act("Fan", WireValue::Ident("green".into()), 43, true),
        Message::read_ok(-273.15, "cpu_temp", false, 7),
        Message::read_ok(f64::MIN_POSITIVE, "attention", true, 8),
        Message::read_err(0, 9),
        Message::read_err(3, 10),
        Message::act_ack(AckAct::Delivered, false, 11),
        Message::act_ack(AckAct::Rejected { limit: 2, limit_value: 200.0 }, false, 12),
        Message::act_ack(AckAct::MissingActor, false, 13),
        Message::act_ack(AckAct::Unavailable, false, 14),
        Message::act_ack(AckAct::FallbackExecuted { alternativo: "ReserveFan".into() }, true, 15),
        Message::act_ack(AckAct::FallbackExhausted, false, 16),
        Message::act_ack(AckAct::InvalidValue { reason: "cor desconhecida: 'roxo'".into() }, false, 17),
        Message::heartbeat("Fan", 18),
        Message::heartbeat_ack(true, 19),
        Message::heartbeat_ack(false, 20),
        Message::hello(vec![], 21),
        Message::hello(
            vec![
                DeviceDesc::Sensor {
                    name: "cpu_temp".into(),
                    min: Some(0.0), // mínimo legítimo 0 (não é "não declarado")
                    max: Some(120.0),
                    quantity: "temperature".into(),
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
            "StatusLed",
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

// ══════════════════════════════════════════════════════════════════════════
// Cobertura complementar: Display dos erros, rejeições de encode restantes,
// HELLO com publicação de registro, heartbeat e string com prefixo de 2 bytes.
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn display_de_todos_os_erros_de_schema() {
    assert_eq!(
        SchemaError::FrameExceeded { length: 9000 }.to_string(),
        "frame de 9000 bytes excede o máximo de 8192"
    );
    assert_eq!(
        SchemaError::PayloadTooShort { length: 3 }.to_string(),
        "payload de 3 bytes é menor que o header de 12"
    );
    assert_eq!(SchemaError::InvalidMagic.to_string(), "magic inválido (esperado \"FXP\")");
    assert_eq!(
        SchemaError::UnknownVersion { received: 9 }.to_string(),
        "versão de schema desconhecida: 9 (v1)"
    );
    assert_eq!(
        SchemaError::UnknownOpcode { received: 0xFF }.to_string(),
        "opcode desconhecido: 0xFF"
    );
    assert_eq!(
        SchemaError::InvalidName.to_string(),
        "opcode exige nome simbólico ausente/truncado"
    );
    assert_eq!(SchemaError::MissingField.to_string(), "corpo da mensagem truncado");
    assert_eq!(SchemaError::PayloadTooLong.to_string(), "bytes excedentes após o corpo");
    assert_eq!(
        SchemaError::ReservedFlag.to_string(),
        "bits reservados de flags devem ser 0 no encode"
    );
    assert_eq!(
        SchemaError::StringTooLong.to_string(),
        "string excede o limite (nome ≤ 255, texto ≤ 1024)"
    );
    assert_eq!(
        SchemaError::NonFiniteValue.to_string(),
        "valor NaN/infinito não é leitura física válida (FORMAL §4.7)"
    );
    assert_eq!(SchemaError::InvalidUtf8.to_string(), "campo de texto com UTF-8 inválido");
}

#[test]
fn encode_rejeita_opcode_desconhecido_e_flag_reservada() {
    let ruim = Message {
        opcode: 0x7F,
        flags: 0,
        seq: 1,
        name: "cpu_temp".into(),
        timestamp_us: None,
        body: Body::Empty,
    };
    assert!(matches!(
        encode_to_vec(&ruim),
        Err(SchemaError::UnknownOpcode { received: 0x7F })
    ));
    let mut reservado = Message::read("cpu_temp", 1, false);
    reservado.flags = 0b1000_0000;
    assert!(matches!(encode_to_vec(&reservado), Err(SchemaError::ReservedFlag)));
}

#[test]
fn hello_com_publicacao_de_registro_e_heartbeat_roundtrip() {
    let devices = vec![
        DeviceDesc::Sensor {
            name: "cpu_temp".into(),
            min: Some(-40.0),
            max: Some(120.0),
            quantity: "temperatura".into(),
            unit: "°C".into(),
            precision_pct: 1.5,
        },
        DeviceDesc::Actor {
            name: "Fan".into(),
            min: Some(0.0),
            max: Some(255.0),
            safety: Some(200.0),
        },
    ];
    let hello = Message::hello(devices.clone(), 9);
    assert_eq!(decode(&encode_to_vec(&hello).unwrap()).unwrap().0, hello);
    // ack de heartbeat nos dois estados
    for ok in [true, false] {
        let hb = Message::heartbeat_ack(ok, 10);
        assert_eq!(decode(&encode_to_vec(&hb).unwrap()).unwrap().0, hb);
    }
    let hb = Message::heartbeat("cpu_temp", 11);
    assert_eq!(decode(&encode_to_vec(&hb).unwrap()).unwrap().0, hb);
}

#[test]
fn string_longa_usa_prefixo_de_dois_bytes() {
    // texto de 300 bytes (> 255) no reason do InvalidValue → prefixo u16
    let longo = "x".repeat(300);
    let msg = Message::act("Fan", WireValue::Ident(longo.clone()), 1, false);
    let frame = encode_to_vec(&msg).unwrap();
    let volta = decode(&frame).unwrap().0;
    assert_eq!(volta, msg);
    // o texto de 300 bytes está inteiro no frame (prefixo de 2 bytes)
    assert!(frame.len() > HEADER_LEN + 300);
    let _ = (MAX_STRING, MAGIC, VERSION);
}

// ══════════════════════════════════════════════════════════════════════════
// v1.1 — CAPS, AUTH, READ_BATCH e FLAG_TIMESTAMP
// (docs/FXP-SCHEMA-v1.md §4.5–§4.8 e §5; contrato escrito antes do código)
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn v11_novos_opcodes_roadtrip_bit_a_bit() {
    let msgs = vec![
        Message::caps(caps::LZ4 | caps::BATCH | caps::TIMESTAMP, 30),
        Message::caps_ok(caps::BATCH | caps::TIMESTAMP, 31),
        Message::read_batch(vec!["cpu_temp".into(), "cpu_power".into()], 32),
        Message::read_batch_ok(
            vec![
                BatchResult::Ok { value: 45.5, canonical: "cpu_temp".into() },
                BatchResult::Err { reason: reason::INACCESSIBLE },
                BatchResult::Err { reason: reason::TIMEOUT },
            ],
            33,
        ),
        Message::auth_challenge(AUTH_SCHEME_PSK_HMAC_SHA256, [7u8; 32], 34),
        Message::auth_response([7u8; 32], [9u8; 32], 35),
        Message::auth_ok(36),
    ];
    for msg in msgs {
        let bytes = encode_to_vec(&msg).unwrap_or_else(|e| panic!("encode falhou: {e}"));
        let (back, consumed) = decode(&bytes).unwrap_or_else(|e| panic!("decode falhou: {e}"));
        assert_eq!(consumed, bytes.len());
        assert_eq!(back, msg, "roundtrip v1.1 alterou a mensagem");
    }
}

#[test]
fn timestamp_deriva_da_flag_e_occupa_o_lugar_do_doc() {
    let plain_msg = Message::read_ok(70.0, "cpu_temp", false, 40);
    let plain = encode_to_vec(&plain_msg).unwrap();
    // wire default: sem FLAG_TIMESTAMP e sem bytes extras
    assert_eq!(plain[4 + 5] & flag::TIMESTAMP, 0);
    assert_eq!(plain.len(), 4 + HEADER_LEN + 8 + 1 + 8); // +canonical("cpu_temp")

    let stamped =
        Message::read_ok(70.0, "cpu_temp", false, 41).with_timestamp(1_756_845_000_000_123);
    let bytes = encode_to_vec(&stamped).unwrap();
    // flag derivada do campo
    assert_eq!(bytes[4 + 5] & flag::TIMESTAMP, flag::TIMESTAMP);
    // layout: u64 LE entre o header (12 B) e o nome/corpo
    let ts = u64::from_le_bytes([
        bytes[4 + HEADER_LEN],
        bytes[4 + HEADER_LEN + 1],
        bytes[4 + HEADER_LEN + 2],
        bytes[4 + HEADER_LEN + 3],
        bytes[4 + HEADER_LEN + 4],
        bytes[4 + HEADER_LEN + 5],
        bytes[4 + HEADER_LEN + 6],
        bytes[4 + HEADER_LEN + 7],
    ]);
    assert_eq!(ts, 1_756_845_000_000_123);
    assert_eq!(bytes.len(), plain.len() + 8, "timestamp custa exatamente 8 B");
    let (back, _) = decode(&bytes).unwrap();
    assert_eq!(back.timestamp_us, Some(1_756_845_000_000_123));
    let Body::ReadOk { value, .. } = back.body else { panic!("corpo errado") };
    assert_eq!(value, 70.0);
    // truncamento em qualquer byte do frame com timestamp também é erro
    for n in 0..bytes.len() {
        assert!(decode(&bytes[..n]).is_err(), "truncado em {n} decodificou");
    }
}

#[test]
fn golden_wire_default_igual_ao_v1_0() {
    // Aditividade (aceite do plano FXP v1.1): nenhum byte novo no wire default.
    let casos = [
        Message::read("cpu_temp", 1, true),
        Message::read_ok(1.0, "cpu_temp", true, 2),
        Message::read_err(2, 3),
        Message::act("Fan", WireValue::Num(128.0), 4, true),
        Message::act_ack(AckAct::Delivered, false, 5),
        Message::heartbeat("Fan", 6),
        Message::heartbeat_ack(true, 7),
        Message::bye(8),
    ];
    for msg in casos {
        let b = encode_to_vec(&msg).unwrap();
        assert_eq!(b[10], 0, "byte reservado deve ser 0 no wire default");
        assert_eq!(b[9] & 0b1111_0000, 0, "bits novos não podem aparecer sem recurso");
    }
    // Frame v1.0 congelado: READ "cpu_temp" seq=1 com FLAG_ACK.
    let mut esperado = vec![20, 0, 0, 0, b'F', b'X', b'P', 1, 0x01, 0x01, 0, 8, 1, 0, 0, 0];
    esperado.extend_from_slice(b"cpu_temp");
    assert_eq!(encode_to_vec(&Message::read("cpu_temp", 1, true)).unwrap(), esperado);
}

#[test]
fn encode_rejeita_bits_de_recurso_setados_a_mao() {
    // FLAG_TIMESTAMP/FLAG_COMPRESSED são derivados no encode (fonte única:
    // o campo `timestamp_us` e o pedido de compressão do transporte).
    let mut msg = Message::read("s", 1, true);
    msg.flags |= flag::TIMESTAMP;
    assert_eq!(encode_to_vec(&msg), Err(SchemaError::ReservedFlag));
    msg.flags = flag::COMPRESSED;
    assert_eq!(encode_to_vec(&msg), Err(SchemaError::ReservedFlag));
}

#[test]
fn batch_respeita_limites_e_erro_por_item_e_honesto() {
    // 64 = limite aceito.
    let nomes: Vec<String> = (0..MAX_BATCH).map(|i| format!("sensor{i}")).collect();
    let msg = Message::read_batch(nomes, 1);
    let (back, _) = decode(&encode_to_vec(&msg).unwrap()).unwrap();
    assert_eq!(back, msg);
    // 65 e vazio → BatchTooLarge (contrato: 1..=64).
    let sessenta_cinco: Vec<String> = (0..MAX_BATCH + 1).map(|i| format!("s{i}")).collect();
    assert_eq!(
        encode_to_vec(&Message::read_batch(sessenta_cinco, 1)),
        Err(SchemaError::BatchTooLarge)
    );
    assert_eq!(
        encode_to_vec(&Message::read_batch(vec![], 1)),
        Err(SchemaError::BatchTooLarge)
    );
    let resultados: Vec<BatchResult> =
        (0..MAX_BATCH + 1).map(|_| BatchResult::Err { reason: 1 }).collect();
    assert_eq!(
        encode_to_vec(&Message::read_batch_ok(resultados, 1)),
        Err(SchemaError::BatchTooLarge)
    );
    // Razão fora de §4.1 é rejeitada no encode…
    assert_eq!(
        encode_to_vec(&Message::read_batch_ok(vec![BatchResult::Err { reason: 5 }], 1)),
        Err(SchemaError::MissingField)
    );
    // …e no decode (byte de status adulterado à mão: status fica após header+count).
    let mut bytes = encode_to_vec(&Message::read_batch_ok(vec![BatchResult::Err { reason: 1 }], 1)).unwrap();
    bytes[4 + HEADER_LEN + 2] = 9;
    assert_eq!(decode(&bytes).unwrap_err(), SchemaError::MissingField);

    // §4.7: razão 0 (nao_registrado) viaja como tag 4 — o byte 0 do item é
    // o status "ok"; ida e volta preserva a razão.
    let item = Message::read_batch_ok(vec![BatchResult::Err { reason: reason::NOT_REGISTERED }], 1);
    let (back, _) = decode(&encode_to_vec(&item).unwrap()).unwrap();
    assert_eq!(
        back.body,
        Body::ReadBatchOk { results: vec![BatchResult::Err { reason: reason::NOT_REGISTERED }] }
    );
}

#[test]
fn auth_scheme_e_nonce_sao_validados() {
    let ruim = Message::auth_challenge(99, [0u8; 32], 1);
    assert!(matches!(
        encode_to_vec(&ruim),
        Err(SchemaError::UnknownAuthScheme { received: 99 })
    ));
    // Nonce tem exatamente 32 bytes no fio (u16 scheme + 32 B).
    let ok = Message::auth_challenge(AUTH_SCHEME_PSK_HMAC_SHA256, [0xAB; 32], 2);
    let b = encode_to_vec(&ok).unwrap();
    assert_eq!(b.len(), 4 + HEADER_LEN + 2 + 32);
    assert_eq!(&b[4 + HEADER_LEN..4 + HEADER_LEN + 2], &[1, 0], "scheme LE");
}

#[test]
fn caps_reservados_sao_rejeitados_no_encode() {
    let msg = Message::caps(0b0000_0000_0000_1000, 1); // bit 3 é reservado
    assert!(matches!(encode_to_vec(&msg), Err(SchemaError::ReservedCaps)));
}

// ══════════════════════════════════════════════════════════════════════════
// v1.1 §4.8 — Compressão LZ4 do corpo
// ══════════════════════════════════════════════════════════════════════════

/// HELLO grande (região > threshold de 512 B).
fn hello_grande() -> Message {
    let devices: Vec<DeviceDesc> = (0..40)
        .map(|i| DeviceDesc::Sensor {
            name: format!("sensor_numero_{i:02}_do_registro_completo"),
            min: Some(-40.0),
            max: Some(120.0),
            quantity: "temperature".into(),
            unit: "°C".into(),
            precision_pct: 1.5,
        })
        .collect();
    Message::hello(devices, 9)
}

#[test]
fn compressao_roundtrip_e_marca_o_frame() {
    use vbl_fxp::schema::encode_with_compression;
    let hello = hello_grande();
    let plano = encode_to_vec(&hello).unwrap();
    let mut comprimido = Vec::new();
    encode_with_compression(&hello, &mut comprimido).unwrap();

    // Header marca flag + algoritmo.
    assert_eq!(comprimido[4 + 5] & flag::COMPRESSED, flag::COMPRESSED);
    assert_eq!(comprimido[4 + 6], vbl_fxp::schema::compress::ALGO_LZ4);
    // Wire menor que o plano (60+ nomes repetidos comprimem bem).
    assert!(
        comprimido.len() < plano.len(),
        "lz4: {} vs plano {}",
        comprimido.len(),
        plano.len()
    );
    // E o roundtrip devolve a mensagem exata.
    let (back, _) = decode(&comprimido).unwrap();
    assert_eq!(back, hello);
}

#[test]
fn compressao_nao_viaja_abaixo_do_threshold_ou_quando_infla() {
    use vbl_fxp::schema::encode_with_compression;
    // READ pequeno: região < 512 B ⇒ sai plano, sem flag.
    let read = Message::read("cpu_temp", 1, true);
    let mut fora = Vec::new();
    encode_with_compression(&read, &mut fora).unwrap();
    assert_eq!(fora[4 + 5] & flag::COMPRESSED, 0, "frame pequeno não comprime");
    assert_eq!(fora, encode_to_vec(&read).unwrap(), "byte a byte igual ao plano");
}

#[test]
fn compressao_rejeita_algoritmo_desconhecido_e_bomba() {
    // Frame com FLAG_COMPRESSED e algoritmo 9 ⇒ UnknownCompression.
    let hello = hello_grande();
    let mut bytes = encode_to_vec(&hello).unwrap();
    // Forja: length +1 (byte extra), flags bit6, reservado 9, byte extra 0xFF.
    let len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    bytes[0..4].copy_from_slice(&(len + 1).to_le_bytes());
    bytes[4 + 5] |= flag::COMPRESSED;
    bytes[4 + 6] = 9;
    bytes.push(0xFF);
    assert_eq!(
        decode(&bytes).unwrap_err(),
        SchemaError::UnknownCompression { received: 9 }
    );

    // Bomba: blob não-LZ4 de 100 bytes ⇒ DecompressionFailed (nunca executa
    // parcialmente).
    let mut bomba = Vec::new();
    bomba.extend_from_slice(&(12u32 + 100).to_le_bytes());
    bomba.extend_from_slice(&encode_to_vec(&Message::bye(1)).unwrap()[4..4 + 12]);
    bomba[4 + 5] |= flag::COMPRESSED;
    bomba[4 + 6] = 1;
    bomba.extend_from_slice(&[0xAB; 100]);
    assert!(matches!(decode(&bomba), Err(SchemaError::DecompressionFailed)));
}

// ══════════════════════════════════════════════════════════════════════════
// Varredura de robustez do codec: truncamento em TODOS os prefixos, WireValue
// nos 3 sabores e lote com itens de erro — o fio é estrito (§5).
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn truncamento_em_qualquer_prefixo_nunca_panica() {
    let mensagens = vec![
        Message::read("cpu_temp", 1, true),
        Message::read_ok(42.0, "cpu_temp", false, 1),
        Message::read_batch(vec!["a".into(), "b".into()], 1),
        Message::read_batch_ok(
            vec![BatchResult::Ok { value: 1.0, canonical: "a".into() },
                 BatchResult::Err { reason: vbl_fxp::schema::reason::NOT_REGISTERED }],
            1,
        ),
        Message::auth_challenge(vbl_fxp::schema::AUTH_SCHEME_PSK_HMAC_SHA256, [9u8; 32], 1),
        Message::auth_response([8u8; 32], [7u8; 32], 1),
        Message::act("Fan", vbl_fxp::WireValue::Num(50.0), 1, false),
        Message::act("Led", vbl_fxp::WireValue::Str("azul".into()), 1, false),
        Message::act("Led", vbl_fxp::WireValue::Ident("auto".into()), 1, false),
        Message::act_ack(AckAct::Delivered, false, 1),
        Message::hello(Vec::new(), 1),
        Message::bye(1),
        Message::heartbeat("cpu_temp", 1),
    ];
    for m in mensagens {
        let bytes = encode_to_vec(&m).unwrap();
        // Truncar em qualquer tamanho < completo ⇒ erro tipado, nunca pânico
        // nem leitura fora do buffer.
        for corte in 0..bytes.len() {
            assert!(
                decode(&bytes[..corte]).is_err(),
                "truncamento em {} de {} decodificou: {:?}",
                corte,
                bytes.len(),
                m
            );
        }
        // E o frame completo sempre volta.
        let (back, _) = decode(&bytes)
            .unwrap_or_else(|e| panic!("frame completo falhou ({e}): {:?}", m.opcode));
        assert_eq!(back, m);
    }
}

#[test]
fn act_ack_e_reason_cobrem_todos_os_estados() {
    use vbl_fxp::schema::AckAct;
    let acks = vec![
        AckAct::Delivered,
        AckAct::Rejected { limit: 0, limit_value: 10.0 },
        AckAct::Rejected { limit: 1, limit_value: 255.0 },
        AckAct::Rejected { limit: 2, limit_value: 200.0 },
        AckAct::MissingActor,
        AckAct::Unavailable,
        AckAct::InvalidValue { reason: "valor não numérico".into() },
        AckAct::FallbackExecuted { alternativo: "ReserveFan".into() },
        AckAct::FallbackExhausted,
    ];
    for ack in acks {
        let m = Message::act_ack(ack, true, 7);
        let (back, _) = decode(&encode_to_vec(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}

#[test]
fn mensagens_de_erro_do_schema_sao_legiveis() {
    use vbl_fxp::schema::{SchemaError, compress};
    let casos: Vec<(SchemaError, &str)> = vec![
        (SchemaError::NonFiniteValue, "NaN/infinito"),
        (SchemaError::InvalidUtf8, "UTF-8"),
        (SchemaError::UnknownCompression { received: 9 }, "compressão"),
        (SchemaError::ReservedCaps, "reservados"),
        (SchemaError::DecompressionFailed, "bomba"),
    ];
    for (err, pedaco) in casos {
        let msg = err.to_string();
        assert!(msg.contains(pedaco), "{err} ⇒ {msg}");
    }
    let _ = compress::ALGO_LZ4;
}

#[test]
fn hello_com_dispositivos_variados_codifica_e_decodifica_tudo() {
    use vbl_fxp::schema::DeviceDesc;
    // A matriz completa do §4.4: sensor COM limites, sensor SEM limites,
    // ator COM safety e ator sem limites — ida e volta byte a byte.
    let devices = vec![
        DeviceDesc::Sensor {
            name: "temp_a".into(),
            min: Some(0.0),
            max: Some(150.0),
            quantity: "temperature".into(),
            unit: "°C".into(),
            precision_pct: 1.5,
        },
        DeviceDesc::Sensor {
            name: "aberto".into(),
            min: None,
            max: None,
            quantity: "power".into(),
            unit: "W".into(),
            precision_pct: 5.0,
        },
        DeviceDesc::Actor {
            name: "Fan".into(),
            min: Some(0.0),
            max: Some(255.0),
            safety: Some(200.0),
        },
        DeviceDesc::Actor { name: "Led".into(), min: None, max: None, safety: None },
    ];
    let m = Message::hello(devices, 3);
    let (back, _) = decode(&encode_to_vec(&m).unwrap()).unwrap();
    assert_eq!(back, m);
}

#[test]
fn frames_forjados_com_campos_invalidos_falham_tipado() {
    use vbl_fxp::schema::op;
    // helper: frame cru no layout do fio (§4.2):
    // [len u32][MAGIC 4B][ver][opcode][flags][rsv][name_len][seq u32][nome][corpo]
    let cru = |opcode: u8, nome: &str, corpo: &[u8]| {
        let mut f = Vec::with_capacity(12 + nome.len() + corpo.len());
        let length = (12 + nome.len() + corpo.len()) as u32;
        f.extend_from_slice(&length.to_le_bytes());
        f.extend_from_slice(&vbl_fxp::schema::MAGIC);
        f.push(vbl_fxp::schema::VERSION);
        f.push(opcode);
        f.push(0); // flags (sem TIMESTAMP/COMPRESSED)
        f.push(0); // reservado
        f.push(nome.len() as u8);
        f.extend_from_slice(&1u32.to_le_bytes()); // seq
        f.extend_from_slice(nome.as_bytes());
        f.extend_from_slice(corpo);
        f
    };
    // ACT com nome vazio ⇒ InvalidName
    let e = decode(&cru(op::ACT, "", &[0, 0, 0, 0, 0, 0, 0, 0])).unwrap_err();
    assert!(e.to_string().contains("nome"), "{e}");
    // ACT com kind desconhecido (9) ⇒ MissingField
    assert!(decode(&cru(op::ACT, "Fan", &[9])).is_err());
    // ACT_ACK com status desconhecido (9) ⇒ MissingField
    assert!(decode(&cru(op::ACT_ACK, "Fan", &[9])).is_err());
    // HELLO com descritor de kind 9 ⇒ MissingField
    assert!(decode(&cru(op::HELLO, "", &[0, 1, 9])).is_err());
    // AUTH_CHALLENGE com nonce cortado ⇒ MissingField (guard do take)
    assert!(decode(&cru(0x08, "", &[1, 2, 3])).is_err());
}

#[test]
fn hello_com_mais_de_64k_dispositivos_e_recusado() {
    use vbl_fxp::schema::DeviceDesc;
    let devices: Vec<DeviceDesc> = (0..=u16::MAX as usize)
        .map(|i| DeviceDesc::Actor {
            name: format!("a{i}"),
            min: None,
            max: None,
            safety: None,
        })
        .collect();
    let m = Message::hello(devices, 0);
    assert!(encode_to_vec(&m).is_err(), "65.537 descritores devem estourar u16");
}

#[test]
fn opcode_name_cobre_os_opcodes_de_corpo_vazio() {
    use vbl_fxp::schema::op;
    for o in [op::READ, op::HEARTBEAT, op::BYE, op::AUTH_OK] {
        assert!(vbl_fxp::schema::op::name(o).is_some(), "opcode {o} sem nome");
    }
}
