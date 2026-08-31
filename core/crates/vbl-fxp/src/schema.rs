//! Codec do schema de mensagem FXP **v1** — implementação de referência de
//! [`docs/FXP-SCHEMA-v1.md`] (PLAN §3.5: schema definido antes dos drivers).
//!
//! Garantias canônicas do v1:
//! - `encode → decode` é identidade bit a bit (f64 IEEE-754, UTF-8 exato);
//! - `NaN`/`±∞` rejeitados no encode **e** no decode (leitura física inválida
//!   é falha de I/O — FORMAL §4.7 —, nunca valor mágico);
//! - violação de enquadramento (magic, versão, opcode, truncamento, sobras)
//!   produz erro explícito, nunca mensagem parcial silenciosa;
//! - endianness little-endian em todos os inteiros e nos bits do f64.

/// Magic do header: `"FXP"`.
pub const MAGIC: [u8; 3] = *b"FXP";
/// Versão do schema — decodificador rejeita versão desconhecida.
pub const VERSION: u8 = 1;
/// Bytes do header fixo.
pub const HEADER_LEN: usize = 12;
/// Limite do payload (`length` do frame), em bytes.
pub const MAX_PAYLOAD: usize = 8192;
/// Limite do nome simbólico (`name_len: u8`).
pub const MAX_NAME: usize = 255;
/// Limite de strings de valor/grandeza/unidade/motivo.
pub const MAX_STRING: usize = 1024;

/// Opcodes do v1 (docs/FXP-SCHEMA-v1.md §4).
pub mod op {
    pub const READ: u8 = 0x01;
    pub const ACT: u8 = 0x02;
    pub const HEARTBEAT: u8 = 0x03;
    pub const HELLO: u8 = 0x04;
    pub const BYE: u8 = 0x05;
    pub const READ_OK: u8 = 0x81;
    pub const READ_ERR: u8 = 0x82;
    pub const HEARTBEAT_ACK: u8 = 0x83;
    pub const ACT_ACK: u8 = 0x84;

    /// Nome canônico do opcode (diagnósticos e Caderno).
    pub fn name(opcode: u8) -> Option<&'static str> {
        Some(match opcode {
            READ => "READ",
            ACT => "ACT",
            HEARTBEAT => "HEARTBEAT",
            HELLO => "HELLO",
            BYE => "BYE",
            READ_OK => "READ_OK",
            READ_ERR => "READ_ERR",
            HEARTBEAT_ACK => "HEARTBEAT_ACK",
            ACT_ACK => "ACT_ACK",
            _ => return None,
        })
    }
}

/// Flags do header (docs/FXP-SCHEMA-v1.md §5).
pub mod flag {
    /// Remetente exige resposta com o mesmo `seq`.
    pub const ACK: u8 = 1 << 0;
    /// Resposta de erro.
    pub const ERROR: u8 = 1 << 1;
    /// Entrega efetivada por ator alternativo (FORMAL §4.3).
    pub const FALLBACK: u8 = 1 << 2;
    /// Dado de origem simulada (`measurement_status` — FORMAL §4.7).
    pub const SYNTHETIC: u8 = 1 << 3;
    /// Bits reservados: `0` no encode; ignorados no decode.
    pub const RESERVED: u8 = 0b1111_0000;
}

/// Razões canônicas de `READ_ERR` (FORMAL §4.7).
pub mod reason {
    pub const NOT_REGISTERED: u8 = 0;
    pub const INACCESSIBLE: u8 = 1;
    pub const TIMEOUT: u8 = 2;
    pub const BUSY: u8 = 3;
}

/// Erro de codificação/decodificação do schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// `length` do frame excede [`MAX_PAYLOAD`].
    FrameExceeded { length: usize },
    /// Payload menor que o header fixo.
    PayloadTooShort { length: usize },
    /// Magic diferente de `"FXP"`.
    InvalidMagic,
    /// `version` != 1.
    UnknownVersion { received: u8 },
    /// Opcode fora da tabela do v1.
    UnknownOpcode { received: u8 },
    /// Opcode exige `name`, ausente ou além do payload.
    InvalidName,
    /// Campo obrigatório ausente/truncado no corpo.
    MissingField,
    /// Bytes sobrando após o corpo completo.
    PayloadTooLong,
    /// Bits reservados de flags diferentes de 0 no encode (v1 é estrito).
    ReservedFlag,
    /// String excede [`MAX_STRING`] (ou nome excede [`MAX_NAME`]).
    StringTooLong,
    /// `NaN`/`±∞` (encode ou decode).
    NonFiniteValue,
    /// UTF-8 inválido em campo de texto.
    InvalidUtf8,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::FrameExceeded { length } => {
                write!(f, "frame de {length} bytes excede o máximo de {MAX_PAYLOAD}")
            }
            SchemaError::PayloadTooShort { length } => {
                write!(f, "payload de {length} bytes é menor que o header de {HEADER_LEN}")
            }
            SchemaError::InvalidMagic => write!(f, "magic inválido (esperado \"FXP\")"),
            SchemaError::UnknownVersion { received } => {
                write!(f, "versão de schema desconhecida: {received} (v1)"
                )
            }
            SchemaError::UnknownOpcode { received } => {
                write!(f, "opcode desconhecido: 0x{received:02X}")
            }
            SchemaError::InvalidName => write!(f, "opcode exige nome simbólico ausente/truncado"),
            SchemaError::MissingField => write!(f, "corpo da mensagem truncado"),
            SchemaError::PayloadTooLong => write!(f, "bytes excedentes após o corpo"),
            SchemaError::ReservedFlag => write!(f, "bits reservados de flags devem ser 0 no encode"),
            SchemaError::StringTooLong => write!(
                f,
                "string excede o limite (nome ≤ {MAX_NAME}, texto ≤ {MAX_STRING})"
            ),
            SchemaError::NonFiniteValue => {
                write!(f, "valor NaN/infinito não é leitura física válida (FORMAL §4.7)")
            }
            SchemaError::InvalidUtf8 => write!(f, "campo de texto com UTF-8 inválido"),
        }
    }
}

impl std::error::Error for SchemaError {}

/// Valor de comando no fio — espelha `vbl_runtime::fxp::Value` preservando a
/// distinção `Str` × `Ident` da AST.
#[derive(Debug, Clone, PartialEq)]
pub enum WireValue {
    Num(f64),
    Str(String),
    Ident(String),
}

/// Status de `ACT_ACK` — espelha 1:1 o `ActOutcome` do runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum AckAct {
    Delivered,
    Rejected { limit: u8, limit_value: f64 },
    MissingActor,
    Unavailable,
    FallbackExecuted { alternativo: String },
    FallbackExhausted,
    InvalidValue { reason: String },
}

/// Descritor de dispositivo do `HELLO` (publicação de registro).
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceDesc {
    Sensor {
        name: String,
        min: Option<f64>,
        max: Option<f64>,
        quantity: String,
        unit: String,
        /// Percentual; `0.0` = não declarado.
        precision_pct: f64,
    },
    Actor {
        name: String,
        min: Option<f64>,
        max: Option<f64>,
        safety: Option<f64>,
    },
}

impl DeviceDesc {
    pub fn name(&self) -> &str {
        match self {
            DeviceDesc::Sensor { name, .. } | DeviceDesc::Actor { name, .. } => name,
        }
    }
}

/// Corpo específico por opcode.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Empty,
    ReadOk { value: f64, canonical: String },
    ReadErr { reason: u8 },
    Act { value: WireValue },
    ActAck { status: AckAct },
    HeartbeatAck { ok: bool },
    Hello { devices: Vec<DeviceDesc> },
}

/// Mensagem FXP v1 (header + nome + corpo).
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub opcode: u8,
    pub flags: u8,
    pub seq: u32,
    /// Nome simbólico (sensor/ator); vazio quando a op não tem nome.
    pub name: String,
    pub body: Body,
}

impl Message {
    /// `READ` de sensor (alias permitido — FORMAL §6).
    pub fn read(sensor: &str, seq: u32, ack: bool) -> Self {
        Self {
            opcode: op::READ,
            flags: if ack { flag::ACK } else { 0 },
            seq,
            name: sensor.into(),
            body: Body::Empty,
        }
    }

    /// `READ_OK` com o nome canônico resolvido e a marca de origem sintética.
    pub fn read_ok(value: f64, canonical: &str, synthetic: bool, seq: u32) -> Self {
        Self {
            opcode: op::READ_OK,
            flags: (if synthetic { flag::SYNTHETIC } else { 0 }) | flag::ACK,
            seq,
            name: String::new(),
            body: Body::ReadOk { value, canonical: canonical.into() },
        }
    }

    /// `READ_ERR` — falha de I/O; **nunca** vale leitura `0.0` (§4.7).
    pub fn read_err(reason: u8, seq: u32) -> Self {
        Self {
            opcode: op::READ_ERR,
            flags: flag::ERROR | flag::ACK,
            seq,
            name: String::new(),
            body: Body::ReadErr { reason },
        }
    }

    /// `ACT` a ator.
    pub fn act(actor: &str, value: WireValue, seq: u32, ack: bool) -> Self {
        Self {
            opcode: op::ACT,
            flags: if ack { flag::ACK } else { 0 },
            seq,
            name: actor.into(),
            body: Body::Act { value },
        }
    }

    /// `ACT_ACK` (status espelha `ActOutcome`).
    pub fn act_ack(status: AckAct, fallback: bool, seq: u32) -> Self {
        Self {
            opcode: op::ACT_ACK,
            flags: flag::ACK | if fallback { flag::FALLBACK } else { 0 },
            seq,
            name: String::new(),
            body: Body::ActAck { status },
        }
    }

    /// `HEARTBEAT` de sondagem.
    pub fn heartbeat(name: &str, seq: u32) -> Self {
        Self {
            opcode: op::HEARTBEAT,
            flags: flag::ACK,
            seq,
            name: name.into(),
            body: Body::Empty,
        }
    }

    /// `HEARTBEAT_ACK`.
    pub fn heartbeat_ack(ok: bool, seq: u32) -> Self {
        Self {
            opcode: op::HEARTBEAT_ACK,
            flags: flag::ACK | if ok { 0 } else { flag::ERROR },
            seq,
            name: String::new(),
            body: Body::HeartbeatAck { ok },
        }
    }

    /// `HELLO` — publicação do registro do peer.
    pub fn hello(devices: Vec<DeviceDesc>, seq: u32) -> Self {
        Self {
            opcode: op::HELLO,
            flags: 0,
            seq,
            name: String::new(),
            body: Body::Hello { devices },
        }
    }

    /// `BYE` — encerramento limpo.
    pub fn bye(seq: u32) -> Self {
        Self { opcode: op::BYE, flags: 0, seq, name: String::new(), body: Body::Empty }
    }
}

// ---------------------------------------------------------------------------
// Escrita de primitivas (LE)
// ---------------------------------------------------------------------------
fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_f64(out: &mut Vec<u8>, v: f64) -> Result<(), SchemaError> {
    if !v.is_finite() {
        return Err(SchemaError::NonFiniteValue);
    }
    out.extend_from_slice(&v.to_bits().to_le_bytes());
    Ok(())
}

fn put_len_string(
    out: &mut Vec<u8>,
    len_bytes: usize,
    limit: usize,
    s: &str,
) -> Result<(), SchemaError> {
    let bytes = s.as_bytes();
    if bytes.len() > limit {
        return Err(SchemaError::StringTooLong);
    }
    match len_bytes {
        1 => out.push(bytes.len() as u8),
        2 => put_u16(out, bytes.len() as u16),
        _ => unreachable!("tamanho do prefixo de string é 1 ou 2"),
    }
    out.extend_from_slice(bytes);
    Ok(())
}

/// Empurra os f64 opcionais para `buf` (fora de ordem) marcando os bits em
/// `flags`; quem chama escreve `flags` ANTES de anexar `buf` no corpo.
fn push_opts(buf: &mut Vec<u8>, flags: &mut u8, pares: &[(u8, Option<f64>)]) -> Result<(), SchemaError> {
    for (bit, v) in pares {
        if let Some(x) = v {
            *flags |= bit;
            put_f64(buf, *x)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------
/// Serializa a mensagem como frame completo (length-prefix + payload) em `out`.
pub fn encode(msg: &Message, out: &mut Vec<u8>) -> Result<(), SchemaError> {
    if op::name(msg.opcode).is_none() {
        return Err(SchemaError::UnknownOpcode { received: msg.opcode });
    }
    if msg.flags & flag::RESERVED != 0 {
        return Err(SchemaError::ReservedFlag);
    }
    if msg.name.len() > MAX_NAME {
        return Err(SchemaError::StringTooLong);
    }

    let mut body = Vec::with_capacity(64);
    match &msg.body {
        Body::Empty => {}
        Body::ReadOk { value, canonical } => {
            put_f64(&mut body, *value)?;
            put_len_string(&mut body, 1, MAX_NAME, canonical)?;
        }
        Body::ReadErr { reason } => body.push(*reason),
        Body::Act { value } => match value {
            WireValue::Num(n) => {
                body.push(0);
                put_f64(&mut body, *n)?;
            }
            WireValue::Str(s) => {
                body.push(1);
                put_len_string(&mut body, 2, MAX_STRING, s)?;
            }
            WireValue::Ident(s) => {
                body.push(2);
                put_len_string(&mut body, 2, MAX_STRING, s)?;
            }
        },
        Body::ActAck { status } => match status {
            AckAct::Delivered => body.push(0),
            AckAct::Rejected { limit, limit_value } => {
                body.push(1);
                body.push(*limit);
                put_f64(&mut body, *limit_value)?;
            }
            AckAct::MissingActor => body.push(2),
            AckAct::Unavailable => body.push(3),
            AckAct::FallbackExecuted { alternativo } => {
                body.push(4);
                put_len_string(&mut body, 1, MAX_NAME, alternativo)?;
            }
            AckAct::FallbackExhausted => body.push(5),
            AckAct::InvalidValue { reason } => {
                body.push(6);
                put_len_string(&mut body, 1, MAX_STRING, reason)?;
            }
        },
        Body::HeartbeatAck { ok } => body.push(u8::from(*ok)),
        Body::Hello { devices } => {
            if devices.len() > u16::MAX as usize {
                return Err(SchemaError::MissingField);
            }
            put_u16(&mut body, devices.len() as u16);
            for d in devices {
                match d {
                    DeviceDesc::Sensor { name, min, max, quantity, unit, precision_pct } => {
                        body.push(0);
                        put_len_string(&mut body, 1, MAX_NAME, name)?;
                        let mut flags = 0u8;
                        let mut opts = Vec::with_capacity(16);
                        push_opts(&mut opts, &mut flags, &[(1, *min), (2, *max)])?;
                        body.push(flags);
                        body.extend_from_slice(&opts);
                        put_len_string(&mut body, 1, MAX_NAME, quantity)?;
                        put_len_string(&mut body, 1, MAX_NAME, unit)?;
                        put_f64(&mut body, *precision_pct)?;
                    }
                    DeviceDesc::Actor { name, min, max, safety } => {
                        body.push(1);
                        put_len_string(&mut body, 1, MAX_NAME, name)?;
                        let mut flags = 0u8;
                        let mut opts = Vec::with_capacity(24);
                        push_opts(&mut opts, &mut flags, &[(1, *min), (2, *max), (4, *safety)])?;
                        body.push(flags);
                        body.extend_from_slice(&opts);
                    }
                }
            }
        }
    }

    if body.len() > MAX_PAYLOAD {
        return Err(SchemaError::FrameExceeded { length: body.len() });
    }
    let length = HEADER_LEN + msg.name.len() + body.len();
    if length > MAX_PAYLOAD {
        return Err(SchemaError::FrameExceeded { length });
    }

    put_u32(out, length as u32);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(msg.opcode);
    out.push(msg.flags);
    out.push(0); // reservado
    out.push(msg.name.len() as u8);
    put_u32(out, msg.seq);
    out.extend_from_slice(msg.name.as_bytes());
    out.extend_from_slice(&body);
    Ok(())
}

/// Atalho: `encode` para um `Vec` novo.
pub fn encode_to_vec(msg: &Message) -> Result<Vec<u8>, SchemaError> {
    let mut out = Vec::with_capacity(HEADER_LEN + msg.name.len() + 64);
    encode(msg, &mut out)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], SchemaError> {
        if self.pos + n > self.buf.len() {
            return Err(SchemaError::MissingField);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, SchemaError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, SchemaError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn f64(&mut self) -> Result<f64, SchemaError> {
        let b = self.take(8)?;
        let v = f64::from_bits(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]));
        if !v.is_finite() {
            return Err(SchemaError::NonFiniteValue);
        }
        Ok(v)
    }
    /// String com prefixo de comprimento de `len_bytes` bytes e limite próprio.
    fn len_string(&mut self, len_bytes: usize, limit: usize) -> Result<String, SchemaError> {
        let n = match len_bytes {
            1 => self.u8()? as usize,
            2 => self.u16()? as usize,
            _ => unreachable!("tamanho do prefixo de string é 1 ou 2"),
        };
        if n > limit {
            return Err(SchemaError::StringTooLong);
        }
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| SchemaError::InvalidUtf8)
    }
    fn opt_f64(&mut self, flags: u8, bit: u8) -> Result<Option<f64>, SchemaError> {
        if flags & bit != 0 {
            Ok(Some(self.f64()?))
        } else {
            Ok(None)
        }
    }
    fn end(&self) -> Result<(), SchemaError> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(SchemaError::PayloadTooLong)
        }
    }
}

/// Lê o `length` do frame no início de `buf` (para framing em stream).
pub fn peek_frame_len(buf: &[u8]) -> Result<usize, SchemaError> {
    if buf.len() < 4 {
        // Ainda não dá para ler o prefixo: não é erro de schema, é incompletude.
        return Ok(0);
    }
    let length = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if length > MAX_PAYLOAD {
        return Err(SchemaError::FrameExceeded { length });
    }
    Ok(length + 4)
}

/// Decodifica **um** frame a partir de `buf`; devolve a mensagem e o total de
/// bytes consumidos (prefixo + payload). Erro se o payload estiver truncado
/// (`MissingField`) ou sobrar bytes dentro do frame (`PayloadTooLong`).
pub fn decode(buf: &[u8]) -> Result<(Message, usize), SchemaError> {
    let total = peek_frame_len(buf)?;
    if total == 0 || buf.len() < total {
        return Err(SchemaError::MissingField);
    }
    let payload = &buf[4..total];
    if payload.len() < HEADER_LEN {
        return Err(SchemaError::PayloadTooShort { length: payload.len() });
    }
    if payload[0..3] != MAGIC {
        return Err(SchemaError::InvalidMagic);
    }
    if payload[3] != VERSION {
        return Err(SchemaError::UnknownVersion { received: payload[3] });
    }
    let opcode = payload[4];
    let flags = payload[5];
    let name_len = payload[7] as usize;
    let seq = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
    if op::name(opcode).is_none() {
        return Err(SchemaError::UnknownOpcode { received: opcode });
    }

    let mut r = Reader::new(&payload[HEADER_LEN..]);
    // O nome no fio não tem prefixo próprio: seu tamanho é o `name_len` do header.
    let name_bytes = r.take(name_len)?;
    let name = String::from_utf8(name_bytes.to_vec()).map_err(|_| SchemaError::InvalidUtf8)?;

    let body = match opcode {
        op::READ_OK => {
            let value = r.f64()?;
            let canonical = r.len_string(1, MAX_NAME)?;
            r.end()?;
            Body::ReadOk { value, canonical }
        }
        op::READ_ERR => {
            let reason = r.u8()?;
            r.end()?;
            Body::ReadErr { reason }
        }
        op::ACT => {
            if name.is_empty() {
                return Err(SchemaError::InvalidName);
            }
            let kind = r.u8()?;
            let value = match kind {
                0 => WireValue::Num(r.f64()?),
                1 => WireValue::Str(r.len_string(2, MAX_STRING)?),
                2 => WireValue::Ident(r.len_string(2, MAX_STRING)?),
                _ => return Err(SchemaError::MissingField),
            };
            r.end()?;
            Body::Act { value }
        }
        op::ACT_ACK => {
            let status = match r.u8()? {
                0 => AckAct::Delivered,
                1 => {
                    let limit = r.u8()?;
                    let limit_value = r.f64()?;
                    AckAct::Rejected { limit, limit_value }
                }
                2 => AckAct::MissingActor,
                3 => AckAct::Unavailable,
                4 => AckAct::FallbackExecuted { alternativo: r.len_string(1, MAX_NAME)? },
                5 => AckAct::FallbackExhausted,
                6 => AckAct::InvalidValue { reason: r.len_string(1, MAX_STRING)? },
                _ => return Err(SchemaError::MissingField),
            };
            r.end()?;
            Body::ActAck { status }
        }
        op::HEARTBEAT_ACK => {
            let ok = r.u8()?;
            r.end()?;
            Body::HeartbeatAck { ok: ok == 1 }
        }
        op::HELLO => {
            let count = r.u16()? as usize;
            let mut devices = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let kind = r.u8()?;
                let name = r.len_string(1, MAX_NAME)?;
                devices.push(match kind {
                    0 => {
                        let flags = r.u8()?;
                        let min = r.opt_f64(flags, 1)?;
                        let max = r.opt_f64(flags, 2)?;
                        let quantity = r.len_string(1, MAX_NAME)?;
                        let unit = r.len_string(1, MAX_NAME)?;
                        let precision_pct = r.f64()?;
                        DeviceDesc::Sensor { name, min, max, quantity, unit, precision_pct }
                    }
                    1 => {
                        let flags = r.u8()?;
                        let min = r.opt_f64(flags, 1)?;
                        let max = r.opt_f64(flags, 2)?;
                        let safety = r.opt_f64(flags, 4)?;
                        DeviceDesc::Actor { name, min, max, safety }
                    }
                    _ => return Err(SchemaError::MissingField),
                });
            }
            r.end()?;
            Body::Hello { devices }
        }
        // Corpos vazios por último (READ exige nome; HEARTBEAT/BYE não).
        op::READ | op::HEARTBEAT | op::BYE => {
            if matches!(opcode, op::READ) && name.is_empty() {
                return Err(SchemaError::InvalidName);
            }
            r.end()?;
            Body::Empty
        }
        _ => unreachable!("opcode validado acima"),
    };

    Ok((Message { opcode, flags, seq, name, body }, total))
}
