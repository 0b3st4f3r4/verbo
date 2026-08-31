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
    pub fn nome(opcode: u8) -> Option<&'static str> {
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
    pub const ERRO: u8 = 1 << 1;
    /// Entrega efetivada por ator alternativo (FORMAL §4.3).
    pub const FALLBACK: u8 = 1 << 2;
    /// Dado de origem simulada (`measurement_status` — FORMAL §4.7).
    pub const SINTETICO: u8 = 1 << 3;
    /// Bits reservados: `0` no encode; ignorados no decode.
    pub const RESERVADOS: u8 = 0b1111_0000;
}

/// Razões canônicas de `READ_ERR` (FORMAL §4.7).
pub mod razao {
    pub const NAO_REGISTRADO: u8 = 0;
    pub const INACESSIVEL: u8 = 1;
    pub const TIMEOUT: u8 = 2;
    pub const OCUPADO: u8 = 3;
}

/// Erro de codificação/decodificação do schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroSchema {
    /// `length` do frame excede [`MAX_PAYLOAD`].
    FrameExcedido { length: usize },
    /// Payload menor que o header fixo.
    PayloadCurto { length: usize },
    /// Magic diferente de `"FXP"`.
    MagicInvalido,
    /// `version` != 1.
    VersaoDesconhecida { recebida: u8 },
    /// Opcode fora da tabela do v1.
    OpcodeDesconhecido { recebido: u8 },
    /// Opcode exige `name`, ausente ou além do payload.
    NomeInvalido,
    /// Campo obrigatório ausente/truncado no corpo.
    CampoFaltante,
    /// Bytes sobrando após o corpo completo.
    PayloadExcedente,
    /// Bits reservados de flags diferentes de 0 no encode (v1 é estrito).
    FlagReservada,
    /// String excede [`MAX_STRING`] (ou nome excede [`MAX_NAME`]).
    StringExcedida,
    /// `NaN`/`±∞` (encode ou decode).
    ValorNaoFinito,
    /// UTF-8 inválido em campo de texto.
    Utf8Invalido,
}

impl std::fmt::Display for ErroSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErroSchema::FrameExcedido { length } => {
                write!(f, "frame de {length} bytes excede o máximo de {MAX_PAYLOAD}")
            }
            ErroSchema::PayloadCurto { length } => {
                write!(f, "payload de {length} bytes é menor que o header de {HEADER_LEN}")
            }
            ErroSchema::MagicInvalido => write!(f, "magic inválido (esperado \"FXP\")"),
            ErroSchema::VersaoDesconhecida { recebida } => {
                write!(f, "versão de schema desconhecida: {recebida} (v1)"
                )
            }
            ErroSchema::OpcodeDesconhecido { recebido } => {
                write!(f, "opcode desconhecido: 0x{recebido:02X}")
            }
            ErroSchema::NomeInvalido => write!(f, "opcode exige nome simbólico ausente/truncado"),
            ErroSchema::CampoFaltante => write!(f, "corpo da mensagem truncado"),
            ErroSchema::PayloadExcedente => write!(f, "bytes excedentes após o corpo"),
            ErroSchema::FlagReservada => write!(f, "bits reservados de flags devem ser 0 no encode"),
            ErroSchema::StringExcedida => write!(
                f,
                "string excede o limite (nome ≤ {MAX_NAME}, texto ≤ {MAX_STRING})"
            ),
            ErroSchema::ValorNaoFinito => {
                write!(f, "valor NaN/infinito não é leitura física válida (FORMAL §4.7)")
            }
            ErroSchema::Utf8Invalido => write!(f, "campo de texto com UTF-8 inválido"),
        }
    }
}

impl std::error::Error for ErroSchema {}

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
    Entregue,
    Rejeitado { limite: u8, valor_limite: f64 },
    AtorInexistente,
    Indisponivel,
    FallbackExecutado { alternativo: String },
    FallbackEsgotado,
    ValorInvalido { motivo: String },
}

/// Descritor de dispositivo do `HELLO` (publicação de registro).
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceDesc {
    Sensor {
        name: String,
        min: Option<f64>,
        max: Option<f64>,
        grandeza: String,
        unidade: String,
        /// Percentual; `0.0` = não declarado.
        precisao_pct: f64,
    },
    Ator {
        name: String,
        min: Option<f64>,
        max: Option<f64>,
        safety: Option<f64>,
    },
}

impl DeviceDesc {
    pub fn name(&self) -> &str {
        match self {
            DeviceDesc::Sensor { name, .. } | DeviceDesc::Ator { name, .. } => name,
        }
    }
}

/// Corpo específico por opcode.
#[derive(Debug, Clone, PartialEq)]
pub enum Corpo {
    Vazio,
    ReadOk { valor: f64, canonical: String },
    ReadErr { reason: u8 },
    Act { valor: WireValue },
    ActAck { status: AckAct },
    HeartbeatAck { ok: bool },
    Hello { devices: Vec<DeviceDesc> },
}

/// Mensagem FXP v1 (header + nome + corpo).
#[derive(Debug, Clone, PartialEq)]
pub struct Mensagem {
    pub opcode: u8,
    pub flags: u8,
    pub seq: u32,
    /// Nome simbólico (sensor/ator); vazio quando a op não tem nome.
    pub name: String,
    pub corpo: Corpo,
}

impl Mensagem {
    /// `READ` de sensor (alias permitido — FORMAL §6).
    pub fn read(sensor: &str, seq: u32, ack: bool) -> Self {
        Self {
            opcode: op::READ,
            flags: if ack { flag::ACK } else { 0 },
            seq,
            name: sensor.into(),
            corpo: Corpo::Vazio,
        }
    }

    /// `READ_OK` com o nome canônico resolvido e a marca de origem sintética.
    pub fn read_ok(valor: f64, canonical: &str, sintetico: bool, seq: u32) -> Self {
        Self {
            opcode: op::READ_OK,
            flags: (if sintetico { flag::SINTETICO } else { 0 }) | flag::ACK,
            seq,
            name: String::new(),
            corpo: Corpo::ReadOk { valor, canonical: canonical.into() },
        }
    }

    /// `READ_ERR` — falha de I/O; **nunca** vale leitura `0.0` (§4.7).
    pub fn read_err(reason: u8, seq: u32) -> Self {
        Self {
            opcode: op::READ_ERR,
            flags: flag::ERRO | flag::ACK,
            seq,
            name: String::new(),
            corpo: Corpo::ReadErr { reason },
        }
    }

    /// `ACT` a ator.
    pub fn act(ator: &str, valor: WireValue, seq: u32, ack: bool) -> Self {
        Self {
            opcode: op::ACT,
            flags: if ack { flag::ACK } else { 0 },
            seq,
            name: ator.into(),
            corpo: Corpo::Act { valor },
        }
    }

    /// `ACT_ACK` (status espelha `ActOutcome`).
    pub fn act_ack(status: AckAct, fallback: bool, seq: u32) -> Self {
        Self {
            opcode: op::ACT_ACK,
            flags: flag::ACK | if fallback { flag::FALLBACK } else { 0 },
            seq,
            name: String::new(),
            corpo: Corpo::ActAck { status },
        }
    }

    /// `HEARTBEAT` de sondagem.
    pub fn heartbeat(nome: &str, seq: u32) -> Self {
        Self {
            opcode: op::HEARTBEAT,
            flags: flag::ACK,
            seq,
            name: nome.into(),
            corpo: Corpo::Vazio,
        }
    }

    /// `HEARTBEAT_ACK`.
    pub fn heartbeat_ack(ok: bool, seq: u32) -> Self {
        Self {
            opcode: op::HEARTBEAT_ACK,
            flags: flag::ACK | if ok { 0 } else { flag::ERRO },
            seq,
            name: String::new(),
            corpo: Corpo::HeartbeatAck { ok },
        }
    }

    /// `HELLO` — publicação do registro do peer.
    pub fn hello(devices: Vec<DeviceDesc>, seq: u32) -> Self {
        Self {
            opcode: op::HELLO,
            flags: 0,
            seq,
            name: String::new(),
            corpo: Corpo::Hello { devices },
        }
    }

    /// `BYE` — encerramento limpo.
    pub fn bye(seq: u32) -> Self {
        Self { opcode: op::BYE, flags: 0, seq, name: String::new(), corpo: Corpo::Vazio }
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

fn put_f64(out: &mut Vec<u8>, v: f64) -> Result<(), ErroSchema> {
    if !v.is_finite() {
        return Err(ErroSchema::ValorNaoFinito);
    }
    out.extend_from_slice(&v.to_bits().to_le_bytes());
    Ok(())
}

fn put_len_string(
    out: &mut Vec<u8>,
    len_bytes: usize,
    limite: usize,
    s: &str,
) -> Result<(), ErroSchema> {
    let bytes = s.as_bytes();
    if bytes.len() > limite {
        return Err(ErroSchema::StringExcedida);
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
fn push_opts(buf: &mut Vec<u8>, flags: &mut u8, pares: &[(u8, Option<f64>)]) -> Result<(), ErroSchema> {
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
pub fn encode(msg: &Mensagem, out: &mut Vec<u8>) -> Result<(), ErroSchema> {
    if op::nome(msg.opcode).is_none() {
        return Err(ErroSchema::OpcodeDesconhecido { recebido: msg.opcode });
    }
    if msg.flags & flag::RESERVADOS != 0 {
        return Err(ErroSchema::FlagReservada);
    }
    if msg.name.len() > MAX_NAME {
        return Err(ErroSchema::StringExcedida);
    }

    let mut corpo = Vec::with_capacity(64);
    match &msg.corpo {
        Corpo::Vazio => {}
        Corpo::ReadOk { valor, canonical } => {
            put_f64(&mut corpo, *valor)?;
            put_len_string(&mut corpo, 1, MAX_NAME, canonical)?;
        }
        Corpo::ReadErr { reason } => corpo.push(*reason),
        Corpo::Act { valor } => match valor {
            WireValue::Num(n) => {
                corpo.push(0);
                put_f64(&mut corpo, *n)?;
            }
            WireValue::Str(s) => {
                corpo.push(1);
                put_len_string(&mut corpo, 2, MAX_STRING, s)?;
            }
            WireValue::Ident(s) => {
                corpo.push(2);
                put_len_string(&mut corpo, 2, MAX_STRING, s)?;
            }
        },
        Corpo::ActAck { status } => match status {
            AckAct::Entregue => corpo.push(0),
            AckAct::Rejeitado { limite, valor_limite } => {
                corpo.push(1);
                corpo.push(*limite);
                put_f64(&mut corpo, *valor_limite)?;
            }
            AckAct::AtorInexistente => corpo.push(2),
            AckAct::Indisponivel => corpo.push(3),
            AckAct::FallbackExecutado { alternativo } => {
                corpo.push(4);
                put_len_string(&mut corpo, 1, MAX_NAME, alternativo)?;
            }
            AckAct::FallbackEsgotado => corpo.push(5),
            AckAct::ValorInvalido { motivo } => {
                corpo.push(6);
                put_len_string(&mut corpo, 1, MAX_STRING, motivo)?;
            }
        },
        Corpo::HeartbeatAck { ok } => corpo.push(u8::from(*ok)),
        Corpo::Hello { devices } => {
            if devices.len() > u16::MAX as usize {
                return Err(ErroSchema::CampoFaltante);
            }
            put_u16(&mut corpo, devices.len() as u16);
            for d in devices {
                match d {
                    DeviceDesc::Sensor { name, min, max, grandeza, unidade, precisao_pct } => {
                        corpo.push(0);
                        put_len_string(&mut corpo, 1, MAX_NAME, name)?;
                        let mut flags = 0u8;
                        let mut opts = Vec::with_capacity(16);
                        push_opts(&mut opts, &mut flags, &[(1, *min), (2, *max)])?;
                        corpo.push(flags);
                        corpo.extend_from_slice(&opts);
                        put_len_string(&mut corpo, 1, MAX_NAME, grandeza)?;
                        put_len_string(&mut corpo, 1, MAX_NAME, unidade)?;
                        put_f64(&mut corpo, *precisao_pct)?;
                    }
                    DeviceDesc::Ator { name, min, max, safety } => {
                        corpo.push(1);
                        put_len_string(&mut corpo, 1, MAX_NAME, name)?;
                        let mut flags = 0u8;
                        let mut opts = Vec::with_capacity(24);
                        push_opts(&mut opts, &mut flags, &[(1, *min), (2, *max), (4, *safety)])?;
                        corpo.push(flags);
                        corpo.extend_from_slice(&opts);
                    }
                }
            }
        }
    }

    if corpo.len() > MAX_PAYLOAD {
        return Err(ErroSchema::FrameExcedido { length: corpo.len() });
    }
    let length = HEADER_LEN + msg.name.len() + corpo.len();
    if length > MAX_PAYLOAD {
        return Err(ErroSchema::FrameExcedido { length });
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
    out.extend_from_slice(&corpo);
    Ok(())
}

/// Atalho: `encode` para um `Vec` novo.
pub fn encode_to_vec(msg: &Mensagem) -> Result<Vec<u8>, ErroSchema> {
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
    fn take(&mut self, n: usize) -> Result<&'a [u8], ErroSchema> {
        if self.pos + n > self.buf.len() {
            return Err(ErroSchema::CampoFaltante);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, ErroSchema> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ErroSchema> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn f64(&mut self) -> Result<f64, ErroSchema> {
        let b = self.take(8)?;
        let v = f64::from_bits(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]));
        if !v.is_finite() {
            return Err(ErroSchema::ValorNaoFinito);
        }
        Ok(v)
    }
    /// String com prefixo de comprimento de `len_bytes` bytes e limite próprio.
    fn len_string(&mut self, len_bytes: usize, limite: usize) -> Result<String, ErroSchema> {
        let n = match len_bytes {
            1 => self.u8()? as usize,
            2 => self.u16()? as usize,
            _ => unreachable!("tamanho do prefixo de string é 1 ou 2"),
        };
        if n > limite {
            return Err(ErroSchema::StringExcedida);
        }
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| ErroSchema::Utf8Invalido)
    }
    fn opt_f64(&mut self, flags: u8, bit: u8) -> Result<Option<f64>, ErroSchema> {
        if flags & bit != 0 {
            Ok(Some(self.f64()?))
        } else {
            Ok(None)
        }
    }
    fn fim(&self) -> Result<(), ErroSchema> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(ErroSchema::PayloadExcedente)
        }
    }
}

/// Lê o `length` do frame no início de `buf` (para framing em stream).
pub fn peek_frame_len(buf: &[u8]) -> Result<usize, ErroSchema> {
    if buf.len() < 4 {
        // Ainda não dá para ler o prefixo: não é erro de schema, é incompletude.
        return Ok(0);
    }
    let length = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if length > MAX_PAYLOAD {
        return Err(ErroSchema::FrameExcedido { length });
    }
    Ok(length + 4)
}

/// Decodifica **um** frame a partir de `buf`; devolve a mensagem e o total de
/// bytes consumidos (prefixo + payload). Erro se o payload estiver truncado
/// (`CampoFaltante`) ou sobrar bytes dentro do frame (`PayloadExcedente`).
pub fn decode(buf: &[u8]) -> Result<(Mensagem, usize), ErroSchema> {
    let total = peek_frame_len(buf)?;
    if total == 0 || buf.len() < total {
        return Err(ErroSchema::CampoFaltante);
    }
    let payload = &buf[4..total];
    if payload.len() < HEADER_LEN {
        return Err(ErroSchema::PayloadCurto { length: payload.len() });
    }
    if payload[0..3] != MAGIC {
        return Err(ErroSchema::MagicInvalido);
    }
    if payload[3] != VERSION {
        return Err(ErroSchema::VersaoDesconhecida { recebida: payload[3] });
    }
    let opcode = payload[4];
    let flags = payload[5];
    let name_len = payload[7] as usize;
    let seq = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
    if op::nome(opcode).is_none() {
        return Err(ErroSchema::OpcodeDesconhecido { recebido: opcode });
    }

    let mut r = Reader::new(&payload[HEADER_LEN..]);
    // O nome no fio não tem prefixo próprio: seu tamanho é o `name_len` do header.
    let name_bytes = r.take(name_len)?;
    let name = String::from_utf8(name_bytes.to_vec()).map_err(|_| ErroSchema::Utf8Invalido)?;

    let corpo = match opcode {
        op::READ_OK => {
            let valor = r.f64()?;
            let canonical = r.len_string(1, MAX_NAME)?;
            r.fim()?;
            Corpo::ReadOk { valor, canonical }
        }
        op::READ_ERR => {
            let reason = r.u8()?;
            r.fim()?;
            Corpo::ReadErr { reason }
        }
        op::ACT => {
            if name.is_empty() {
                return Err(ErroSchema::NomeInvalido);
            }
            let kind = r.u8()?;
            let valor = match kind {
                0 => WireValue::Num(r.f64()?),
                1 => WireValue::Str(r.len_string(2, MAX_STRING)?),
                2 => WireValue::Ident(r.len_string(2, MAX_STRING)?),
                _ => return Err(ErroSchema::CampoFaltante),
            };
            r.fim()?;
            Corpo::Act { valor }
        }
        op::ACT_ACK => {
            let status = match r.u8()? {
                0 => AckAct::Entregue,
                1 => {
                    let limite = r.u8()?;
                    let valor_limite = r.f64()?;
                    AckAct::Rejeitado { limite, valor_limite }
                }
                2 => AckAct::AtorInexistente,
                3 => AckAct::Indisponivel,
                4 => AckAct::FallbackExecutado { alternativo: r.len_string(1, MAX_NAME)? },
                5 => AckAct::FallbackEsgotado,
                6 => AckAct::ValorInvalido { motivo: r.len_string(1, MAX_STRING)? },
                _ => return Err(ErroSchema::CampoFaltante),
            };
            r.fim()?;
            Corpo::ActAck { status }
        }
        op::HEARTBEAT_ACK => {
            let ok = r.u8()?;
            r.fim()?;
            Corpo::HeartbeatAck { ok: ok == 1 }
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
                        let grandeza = r.len_string(1, MAX_NAME)?;
                        let unidade = r.len_string(1, MAX_NAME)?;
                        let precisao_pct = r.f64()?;
                        DeviceDesc::Sensor { name, min, max, grandeza, unidade, precisao_pct }
                    }
                    1 => {
                        let flags = r.u8()?;
                        let min = r.opt_f64(flags, 1)?;
                        let max = r.opt_f64(flags, 2)?;
                        let safety = r.opt_f64(flags, 4)?;
                        DeviceDesc::Ator { name, min, max, safety }
                    }
                    _ => return Err(ErroSchema::CampoFaltante),
                });
            }
            r.fim()?;
            Corpo::Hello { devices }
        }
        // Corpos vazios por último (READ exige nome; HEARTBEAT/BYE não).
        op::READ | op::HEARTBEAT | op::BYE => {
            if matches!(opcode, op::READ) && name.is_empty() {
                return Err(ErroSchema::NomeInvalido);
            }
            r.fim()?;
            Corpo::Vazio
        }
        _ => unreachable!("opcode validado acima"),
    };

    Ok((Mensagem { opcode, flags, seq, name, corpo }, total))
}
