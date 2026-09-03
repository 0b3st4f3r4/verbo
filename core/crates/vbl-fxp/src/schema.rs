//! Codec do schema de mensagem FXP **v1.1** — implementação de referência de
//! [`docs/FXP-SCHEMA-v1.md`] (PLAN §3.5: schema definido antes dos drivers;
//! v1.1: extensões do fio — PLAN §8 item 8).
//!
//! Garantias canônicas:
//! - `encode → decode` é identidade bit a bit (f64 IEEE-754, UTF-8 exato);
//! - `NaN`/`±∞` rejeitados no encode **e** no decode (leitura física inválida
//!   é falha de I/O — FORMAL §4.7 —, nunca valor mágico);
//! - violação de enquadramento (magic, versão, opcode, truncamento, sobras)
//!   produz erro explícito, nunca mensagem parcial silenciosa;
//! - endianness little-endian em todos os inteiros e nos bits do f64;
//! - **aditivo e negociado (v1.1):** o wire default é bit a bit v1.0; recursos
//!   novos (timestamp §5, compressão §4.8, batch §4.7) só trafegam após `CAPS`
//!   (a negociação em si é do transporte/`peer`, o codec é stateless).

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
/// Limite de itens de um `READ_BATCH` (§4.7: 1..=64).
pub const MAX_BATCH: usize = 64;
/// Bytes do nonce do handshake de autenticação (§4.6).
pub const AUTH_NONCE_LEN: usize = 32;
/// Único scheme de autenticação da v1.1: PSK + HMAC-SHA256 (§4.6).
pub const AUTH_SCHEME_PSK_HMAC_SHA256: u16 = 1;

/// Opcodes (docs/FXP-SCHEMA-v1.md §4; v1.1 acrescenta 0x06–0x09 e acks 0x86+).
pub mod op {
    pub const READ: u8 = 0x01;
    pub const ACT: u8 = 0x02;
    pub const HEARTBEAT: u8 = 0x03;
    pub const HELLO: u8 = 0x04;
    pub const BYE: u8 = 0x05;
    /// Negociação de capacidades (v1.1 §4.5).
    pub const CAPS: u8 = 0x06;
    /// Batching de leituras (v1.1 §4.7).
    pub const READ_BATCH: u8 = 0x07;
    /// Desafio de autenticação PSK (v1.1 §4.6).
    pub const AUTH_CHALLENGE: u8 = 0x08;
    /// Resposta de autenticação PSK (v1.1 §4.6).
    pub const AUTH_RESPONSE: u8 = 0x09;
    pub const READ_OK: u8 = 0x81;
    pub const READ_ERR: u8 = 0x82;
    pub const HEARTBEAT_ACK: u8 = 0x83;
    pub const ACT_ACK: u8 = 0x84;
    /// Concessão de capacidades = interseção (v1.1 §4.5).
    pub const CAPS_OK: u8 = 0x86;
    /// Resposta do lote de leituras (v1.1 §4.7).
    pub const READ_BATCH_OK: u8 = 0x87;
    /// Handshake de autenticação aceito (v1.1 §4.6).
    pub const AUTH_OK: u8 = 0x8A;

    /// Nome canônico do opcode (diagnósticos e Caderno).
    pub fn name(opcode: u8) -> Option<&'static str> {
        Some(match opcode {
            READ => "READ",
            ACT => "ACT",
            HEARTBEAT => "HEARTBEAT",
            HELLO => "HELLO",
            BYE => "BYE",
            CAPS => "CAPS",
            READ_BATCH => "READ_BATCH",
            AUTH_CHALLENGE => "AUTH_CHALLENGE",
            AUTH_RESPONSE => "AUTH_RESPONSE",
            READ_OK => "READ_OK",
            READ_ERR => "READ_ERR",
            HEARTBEAT_ACK => "HEARTBEAT_ACK",
            ACT_ACK => "ACT_ACK",
            CAPS_OK => "CAPS_OK",
            READ_BATCH_OK => "READ_BATCH_OK",
            AUTH_OK => "AUTH_OK",
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
    /// Payload carrega `u64 LE` µs desde o epoch UNIX (v1.1 §5 — anotação de
    /// laboratório; o Caderno permanece no relógio virtual). Derivado no
    /// encode do campo `Message::timestamp_us`.
    pub const TIMESTAMP: u8 = 1 << 5;
    /// Região nome+corpo comprimida em LZ4 block (v1.1 §4.8). Somente com
    /// negociação `CAPS` bit 0; nunca setado à mão no `Message`.
    pub const COMPRESSED: u8 = 1 << 6;
    /// Bits reservados (4 e 7): `0` no encode; ignorados no decode.
    pub const RESERVED: u8 = 0b1001_0000;
}

/// Bits de capacidades negociadas (`CAPS`/`CAPS_OK`, v1.1 §4.5).
pub mod caps {
    /// Frames comprimidos em LZ4 block (§4.8).
    pub const LZ4: u16 = 1 << 0;
    /// Opcodes `READ_BATCH`/`READ_BATCH_OK` (§4.7).
    pub const BATCH: u16 = 1 << 1;
    /// Frames com `FLAG_TIMESTAMP` (§5).
    pub const TIMESTAMP: u16 = 1 << 2;
    /// Dicionário de compressão compartilhado do registro (v1.2/v1.3 §4.8).
    /// Quando concedido, o `HELLO` (§4.4) integra o handshake e ambos os
    /// lados derivam o mesmo dicionário do registro do servidor.
    pub const DICT: u16 = 1 << 3;
    /// zstd com dicionário TREINADO (v1.3 §4.8) — habilita o algoritmo 3.
    /// Sempre negociado JUNTO com `DICT` (o gatilho do `HELLO` é o mesmo);
    /// quem pede zstd pede os dois bits. Bits reservados: 5–15.
    pub const ZSTD: u16 = 1 << 4;
    /// Bits reservados: `0` no encode; ignorados no decode (5–15 desde a
    /// v1.3 — o bit 4 virou `ZSTD`; peers v1.2 o ignoram no decode).
    pub const RESERVED: u16 = !0b11111;
}

/// Algoritmos e política de compressão do corpo (v1.1 §4.8).
pub mod compress {
    pub const ALGO_NONE: u8 = 0;
    /// LZ4 block — único algoritmo da v1.1 (via `lz4_flex`).
    pub const ALGO_LZ4: u8 = 1;
    /// LZ4 block + dicionário compartilhado do registro (v1.2 §4.8) — o
    /// dicionário nunca cruza o fio: cada lado deriva dos nomes canônicos
    /// que já possui (servidor: o próprio registro; cliente: a resposta
    /// `HELLO`). Peer sem o bit `DICT` negociado vê id desconhecido ⇒
    /// `UnknownCompression` (princípio 7 — fail closed).
    pub const ALGO_LZ4_DICT: u8 = 2;
    /// zstd + dicionário TREINADO do registro (v1.3 §4.8) — o dicionário é
    /// treinado (`zstd::dict::from_samples`, COVER) sobre os MESMOS nomes
    /// canônicos nos dois lados; zero bytes de dicionário no fio. Exige os
    /// bits `DICT` + `ZSTD` negociados; divergência de dicionário (ex.:
    /// versões de zstd diferentes nas pontas) ⇒ `DecompressionFailed`
    /// (fail closed — nunca lixo silencioso).
    pub const ALGO_ZSTD_DICT: u8 = 3;
    /// Só comprime quando a região plana excede este tamanho (bytes).
    pub const THRESHOLD: usize = 512;
    /// Teto determinístico do dicionário derivado (64 KiB — acima disso o
    /// LZ4 block praticamente não ganha nada; truncar em ordem ordenada
    /// mantém os dois lados com os MESMOS bytes).
    pub const DICT_MAX: usize = 64 * 1024;
    /// Nível zstd do fio (v1.3 §4.8) — constante da especificação: ambas as
    /// pontas codificam no mesmo nível (a decodificação independe, mas o
    /// determinismo do bench e do "nunca inflar" apreciam).
    pub const ZSTD_LEVEL: i32 = 3;
    /// Teto do dicionário TREINADO (16 KiB): o COVER converte o excedente em
    /// estatística, não em matéria crua — com nomes de registro, 16 KiB de
    /// dicionário já cobre o frame-teto (8 KiB) com folga.
    pub const ZSTD_DICT_MAX: usize = 16 * 1024;

    /// Dicionário compartilhado (v1.2 §4.8): nomes canônicos **ordenados**,
    /// concatenados com `\n`, truncados em [`DICT_MAX`] bytes — a mesma
    /// matéria da impressão digital do beacon (§4.9), agora com nomes
    /// completos (onde a razão de compressão mora entre frames).
    pub fn dict_from_registry(names: &[String]) -> Vec<u8> {
        let mut sorted: Vec<&String> = names.iter().collect();
        sorted.sort();
        let mut out = Vec::new();
        for (i, n) in sorted.iter().enumerate() {
            if i > 0 {
                out.push(b'\n');
            }
            out.extend_from_slice(n.as_bytes());
        }
        out.truncate(DICT_MAX);
        out
    }

    /// Dicionário TREINADO (v1.3 §4.8): COVER sobre os nomes canônicos
    /// **ordenados** (mesma matéria do id 2, agora como amostras). Treino é
    /// determinístico para (nomes, versão do zstd) — pontas com versões
    /// diferentes podem divergir ⇒ `DecompressionFailed` (honesto). Registro
    /// pequeno demais para treinar ⇒ `None` (o servidor não concede `ZSTD`).
    pub fn zstd_dict_from_registry(names: &[String]) -> Option<Vec<u8>> {
        let mut sorted: Vec<&String> = names.iter().collect();
        sorted.sort();
        let total: usize = sorted.iter().map(|n| n.len()).sum();
        if sorted.is_empty() || total == 0 {
            return None;
        }
        // Pede o teto; COVER trunca no que dá. Com corpus curto, pedir menos
        // que o corpus falha ("Destination buffer is too small") — a segunda
        // tentativa com o tamanho do corpus resolve; sem isso, sem zstd.
        match zstd::dict::from_samples(&sorted, ZSTD_DICT_MAX) {
            Ok(d) => Some(d),
            Err(_) => zstd::dict::from_samples(&sorted, total).ok(),
        }
    }

    /// Dicionário da conexão (v1.2/v1.3 §4.8): matéria + algoritmo — o id no
    /// fio só decodifica com o dicionário DO SEU algoritmo (id 2 exige a
    /// matéria concatenada; id 3 exige a treinada; o contrário é
    /// `UnknownCompression` — fail closed por construção).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum DictConexao {
        /// LZ4 block + concatenação dos nomes (id 2, v1.2).
        Lz4(Vec<u8>),
        /// zstd + dicionário treinado (id 3, v1.3).
        Zstd(Vec<u8>),
    }
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
    /// Lote de leitura fora de `1..=MAX_BATCH` (v1.1 §4.7).
    BatchTooLarge,
    /// `AUTH_CHALLENGE.scheme` desconhecido (v1.1 §4.6).
    UnknownAuthScheme { received: u16 },
    /// Algoritmo de compressão no byte reservado ≠ id conhecido (v1.1 §4.8).
    UnknownCompression { received: u8 },
    /// Blob LZ4 corrupto ou região descomprimida acima do teto — bomba de
    /// descompressão (v1.1 §4.8).
    DecompressionFailed,
    /// A compressão do corpo falhou no encode (v1.3 §4.8 — ex.: zstd com
    /// dicionário inválido). Nunca vira frame plano silencioso.
    CompressionFailed,
    /// Bits reservados de capacidades ≠ 0 (v1.1 §4.5).
    ReservedCaps,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::FrameExceeded { length } => {
                write!(
                    f,
                    "frame de {length} bytes excede o máximo de {MAX_PAYLOAD}"
                )
            }
            SchemaError::PayloadTooShort { length } => {
                write!(
                    f,
                    "payload de {length} bytes é menor que o header de {HEADER_LEN}"
                )
            }
            SchemaError::InvalidMagic => write!(f, "magic inválido (esperado \"FXP\")"),
            SchemaError::UnknownVersion { received } => {
                write!(f, "versão de schema desconhecida: {received} (v1)")
            }
            SchemaError::UnknownOpcode { received } => {
                write!(f, "opcode desconhecido: 0x{received:02X}")
            }
            SchemaError::InvalidName => write!(f, "opcode exige nome simbólico ausente/truncado"),
            SchemaError::MissingField => write!(f, "corpo da mensagem truncado"),
            SchemaError::PayloadTooLong => write!(f, "bytes excedentes após o corpo"),
            SchemaError::ReservedFlag => {
                write!(f, "bits reservados de flags devem ser 0 no encode")
            }
            SchemaError::StringTooLong => write!(
                f,
                "string excede o limite (nome ≤ {MAX_NAME}, texto ≤ {MAX_STRING})"
            ),
            SchemaError::NonFiniteValue => {
                write!(
                    f,
                    "valor NaN/infinito não é leitura física válida (FORMAL §4.7)"
                )
            }
            SchemaError::InvalidUtf8 => write!(f, "campo de texto com UTF-8 inválido"),
            SchemaError::BatchTooLarge => {
                write!(f, "lote de leitura fora de 1..={MAX_BATCH} itens (§4.7)")
            }
            SchemaError::UnknownAuthScheme { received } => {
                write!(f, "scheme de autenticação desconhecido: {received} (v1.1: {AUTH_SCHEME_PSK_HMAC_SHA256})")
            }
            SchemaError::UnknownCompression { received } => {
                write!(
                    f,
                    "algoritmo de compressão desconhecido: {received} (v1.1: LZ4={})",
                    compress::ALGO_LZ4
                )
            }
            SchemaError::DecompressionFailed => write!(
                f,
                "blob corrupto ou região descomprimida acima de {MAX_PAYLOAD} bytes (bomba)"
            ),
            SchemaError::CompressionFailed => {
                write!(f, "compressão do corpo falhou no encode (§4.8)")
            }
            SchemaError::ReservedCaps => {
                write!(
                    f,
                    "bits reservados de capacidades devem ser 0 (v1.1: 0..=2)"
                )
            }
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

/// Resultado por item de `READ_BATCH_OK` (v1.1 §4.7): erro espelha as razões
/// de `READ_ERR` — item com falha **nunca** vira valor (FORMAL §4.7).
#[derive(Debug, Clone, PartialEq)]
pub enum BatchResult {
    Ok { value: f64, canonical: String },
    Err { reason: u8 },
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
    ReadOk {
        value: f64,
        canonical: String,
    },
    ReadErr {
        reason: u8,
    },
    Act {
        value: WireValue,
    },
    ActAck {
        status: AckAct,
    },
    HeartbeatAck {
        ok: bool,
    },
    Hello {
        devices: Vec<DeviceDesc>,
    },
    /// Capacidades pedidas (`CAPS`) ou concedidas (`CAPS_OK`) — v1.1 §4.5.
    Caps {
        capabilities: u16,
    },
    /// Lote de leituras — v1.1 §4.7.
    ReadBatch {
        names: Vec<String>,
    },
    /// Resposta do lote — v1.1 §4.7.
    ReadBatchOk {
        results: Vec<BatchResult>,
    },
    /// Desafio PSK — v1.1 §4.6.
    AuthChallenge {
        scheme: u16,
        nonce: [u8; AUTH_NONCE_LEN],
    },
    /// Resposta PSK — v1.1 §4.6.
    AuthResponse {
        nonce: [u8; AUTH_NONCE_LEN],
        mac: [u8; AUTH_NONCE_LEN],
    },
    // `AUTH_OK` usa `Body::Empty` — o opcode distingue.
}

/// Mensagem FXP v1.1 (header + nome + corpo).
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub opcode: u8,
    pub flags: u8,
    pub seq: u32,
    /// Nome simbólico (sensor/ator); vazio quando a op não tem nome.
    pub name: String,
    /// Timestamp físico no fio (µs desde o epoch UNIX, v1.1 §5) — anotação de
    /// laboratório; `Some` deriva `FLAG_TIMESTAMP` no encode. O Caderno segue
    /// no relógio virtual do runtime.
    pub timestamp_us: Option<u64>,
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
            timestamp_us: None,
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
            timestamp_us: None,
            body: Body::ReadOk {
                value,
                canonical: canonical.into(),
            },
        }
    }

    /// `READ_ERR` — falha de I/O; **nunca** vale leitura `0.0` (§4.7).
    pub fn read_err(reason: u8, seq: u32) -> Self {
        Self {
            opcode: op::READ_ERR,
            flags: flag::ERROR | flag::ACK,
            seq,
            name: String::new(),
            timestamp_us: None,
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
            timestamp_us: None,
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
            timestamp_us: None,
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
            timestamp_us: None,
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
            timestamp_us: None,
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
            timestamp_us: None,
            body: Body::Hello { devices },
        }
    }

    /// `BYE` — encerramento limpo.
    pub fn bye(seq: u32) -> Self {
        Self {
            opcode: op::BYE,
            flags: 0,
            seq,
            name: String::new(),
            timestamp_us: None,
            body: Body::Empty,
        }
    }

    // ------------------------------------------------------------------
    // v1.1 (docs/FXP-SCHEMA-v1.md §4.5–§4.7)
    // ------------------------------------------------------------------

    /// `CAPS` — capacidades pedidas pelo consumidor (§4.5).
    pub fn caps(capabilities: u16, seq: u32) -> Self {
        Self {
            opcode: op::CAPS,
            flags: flag::ACK,
            seq,
            name: String::new(),
            timestamp_us: None,
            body: Body::Caps { capabilities },
        }
    }

    /// `CAPS_OK` — interseção pedidos × suportados (§4.5).
    pub fn caps_ok(capabilities: u16, seq: u32) -> Self {
        Self {
            opcode: op::CAPS_OK,
            flags: flag::ACK,
            seq,
            name: String::new(),
            timestamp_us: None,
            body: Body::Caps { capabilities },
        }
    }

    /// `READ_BATCH` — lote de leituras (§4.7); `1..=64` nomes.
    pub fn read_batch(names: Vec<String>, seq: u32) -> Self {
        Self {
            opcode: op::READ_BATCH,
            flags: flag::ACK,
            seq,
            name: String::new(),
            timestamp_us: None,
            body: Body::ReadBatch { names },
        }
    }

    /// `READ_BATCH_OK` — resultado por item, erro honesto (§4.7).
    pub fn read_batch_ok(results: Vec<BatchResult>, seq: u32) -> Self {
        Self {
            opcode: op::READ_BATCH_OK,
            flags: flag::ACK,
            seq,
            name: String::new(),
            timestamp_us: None,
            body: Body::ReadBatchOk { results },
        }
    }

    /// `AUTH_CHALLENGE` — scheme + nonce do servidor (§4.6).
    pub fn auth_challenge(scheme: u16, nonce: [u8; AUTH_NONCE_LEN], seq: u32) -> Self {
        Self {
            opcode: op::AUTH_CHALLENGE,
            flags: 0,
            seq,
            name: String::new(),
            timestamp_us: None,
            body: Body::AuthChallenge { scheme, nonce },
        }
    }

    /// `AUTH_RESPONSE` — nonce do consumidor + HMAC (§4.6).
    pub fn auth_response(nonce: [u8; AUTH_NONCE_LEN], mac: [u8; AUTH_NONCE_LEN], seq: u32) -> Self {
        Self {
            opcode: op::AUTH_RESPONSE,
            flags: flag::ACK,
            seq,
            name: String::new(),
            timestamp_us: None,
            body: Body::AuthResponse { nonce, mac },
        }
    }

    /// `AUTH_OK` — handshake aceito (§4.6).
    pub fn auth_ok(seq: u32) -> Self {
        Self {
            opcode: op::AUTH_OK,
            flags: flag::ACK,
            seq,
            name: String::new(),
            timestamp_us: None,
            body: Body::Empty,
        }
    }

    /// Carimba o instante físico (µs desde o epoch UNIX) — deriva
    /// `FLAG_TIMESTAMP` no encode (§5). Anotação de laboratório.
    pub fn with_timestamp(mut self, us: u64) -> Self {
        self.timestamp_us = Some(us);
        self
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

fn put_u64(out: &mut Vec<u8>, v: u64) {
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
fn push_opts(
    buf: &mut Vec<u8>,
    flags: &mut u8,
    pares: &[(u8, Option<f64>)],
) -> Result<(), SchemaError> {
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
///
/// O wire produzido é **plano** (sem compressão — para isso o transporte usa
/// [`encode_with_compression`]); `FLAG_TIMESTAMP` é **derivado** do campo
/// `timestamp_us` e bits de recurso setados à mão são rejeitados (fonte única
/// da verdade — nunca contradição header × campo).
pub fn encode(msg: &Message, out: &mut Vec<u8>) -> Result<(), SchemaError> {
    if op::name(msg.opcode).is_none() {
        return Err(SchemaError::UnknownOpcode {
            received: msg.opcode,
        });
    }
    // Bits de recurso derivados não podem vir setados no Message.
    if msg.flags & (flag::TIMESTAMP | flag::COMPRESSED) != 0 || msg.flags & flag::RESERVED != 0 {
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
                    DeviceDesc::Sensor {
                        name,
                        min,
                        max,
                        quantity,
                        unit,
                        precision_pct,
                    } => {
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
                    DeviceDesc::Actor {
                        name,
                        min,
                        max,
                        safety,
                    } => {
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
        Body::Caps { capabilities } => {
            if capabilities & caps::RESERVED != 0 {
                return Err(SchemaError::ReservedCaps);
            }
            put_u16(&mut body, *capabilities);
        }
        Body::ReadBatch { names } => {
            if names.is_empty() || names.len() > MAX_BATCH {
                return Err(SchemaError::BatchTooLarge);
            }
            put_u16(&mut body, names.len() as u16);
            for n in names {
                put_len_string(&mut body, 1, MAX_NAME, n)?;
            }
        }
        Body::ReadBatchOk { results } => {
            if results.is_empty() || results.len() > MAX_BATCH {
                return Err(SchemaError::BatchTooLarge);
            }
            put_u16(&mut body, results.len() as u16);
            for r in results {
                match r {
                    BatchResult::Ok { value, canonical } => {
                        body.push(0);
                        put_f64(&mut body, *value)?;
                        put_len_string(&mut body, 1, MAX_NAME, canonical)?;
                    }
                    BatchResult::Err { reason } => {
                        // Razão 0 (nao_registrado) viaja como tag 4: o byte 0
                        // do item é o status "ok" (§4.7) — nunca colidir.
                        body.push(match *reason {
                            reason::NOT_REGISTERED => 4,
                            r @ reason::INACCESSIBLE..=reason::BUSY => r,
                            _ => return Err(SchemaError::MissingField),
                        });
                    }
                }
            }
        }
        Body::AuthChallenge { scheme, nonce } => {
            if *scheme != AUTH_SCHEME_PSK_HMAC_SHA256 {
                return Err(SchemaError::UnknownAuthScheme { received: *scheme });
            }
            put_u16(&mut body, *scheme);
            body.extend_from_slice(nonce);
        }
        Body::AuthResponse { nonce, mac } => {
            body.extend_from_slice(nonce);
            body.extend_from_slice(mac);
        }
    }

    // FLAG_TIMESTAMP é derivado do campo (fonte única — §5).
    let effective_flags = msg.flags
        | if msg.timestamp_us.is_some() {
            flag::TIMESTAMP
        } else {
            0
        };
    let ts_len = if msg.timestamp_us.is_some() { 8 } else { 0 };
    let length = HEADER_LEN + ts_len + msg.name.len() + body.len();
    if length > MAX_PAYLOAD {
        return Err(SchemaError::FrameExceeded { length });
    }

    put_u32(out, length as u32);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(msg.opcode);
    out.push(effective_flags);
    out.push(0); // reservado (compressão só via encode_with_compression, §4.8)
    out.push(msg.name.len() as u8);
    put_u32(out, msg.seq);
    if let Some(ts) = msg.timestamp_us {
        put_u64(out, ts);
    }
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

/// Encode com compressão LZ4 do corpo **quando compensa** (§4.8): só quando a
/// região nome+corpo excede [`compress::THRESHOLD`] e o blob fica menor que a
/// região plana (compressão que infla não viaja). Marca `FLAG_COMPRESSED` +
/// id do algoritmo no byte reservado. A negociação de capacidade (`CAPS` bit 0)
/// é responsabilidade do transporte/`peer` — codec é stateless.
pub fn encode_with_compression(msg: &Message, out: &mut Vec<u8>) -> Result<(), SchemaError> {
    let mut plain = Vec::with_capacity(HEADER_LEN + msg.name.len() + 64);
    encode(msg, &mut plain)?;
    let ts_len = usize::from(msg.timestamp_us.is_some());
    let prefix = HEADER_LEN + 8 * ts_len;
    let region = &plain[4 + prefix..];
    if region.len() <= compress::THRESHOLD {
        out.extend_from_slice(&plain);
        return Ok(());
    }
    let blob = lz4_flex::block::compress(region);
    if blob.len() >= region.len() {
        out.extend_from_slice(&plain);
        return Ok(());
    }
    let length = prefix + blob.len();
    if length > MAX_PAYLOAD {
        return Err(SchemaError::FrameExceeded { length });
    }
    out.extend_from_slice(&plain[..4 + prefix]);
    // Patch do frame já copiado: novo `length`, flag e id do algoritmo.
    out[0..4].copy_from_slice(&(length as u32).to_le_bytes());
    out[4 + 5] |= flag::COMPRESSED;
    out[4 + 6] = compress::ALGO_LZ4;
    out.extend_from_slice(&blob);
    Ok(())
}

/// Encode com o dicionário compartilhado (v1.2 §4.8): mesmas regras do
/// [`encode_with_compression`] (threshold, nunca inflar, teto), com o
/// algoritmo `ALGO_LZ4_DICT` no byte reservado. O dicionário é a matéria
/// derivada do registro do servidor em AMBOS os lados
/// ([`compress::dict_from_registry`]) — só quem negociou `caps::DICT` e já
/// completou o `HELLO` consegue decodificar; os demais falham fechado.
pub fn encode_with_compression_dict(
    msg: &Message,
    dict: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), SchemaError> {
    let mut plain = Vec::with_capacity(HEADER_LEN + msg.name.len() + 64);
    encode(msg, &mut plain)?;
    let ts_len = usize::from(msg.timestamp_us.is_some());
    let prefix = HEADER_LEN + 8 * ts_len;
    let region = &plain[4 + prefix..];
    if region.len() <= compress::THRESHOLD {
        out.extend_from_slice(&plain);
        return Ok(());
    }
    let blob = lz4_flex::block::compress_with_dict(region, dict);
    if blob.len() >= region.len() {
        out.extend_from_slice(&plain);
        return Ok(());
    }
    let length = prefix + blob.len();
    if length > MAX_PAYLOAD {
        return Err(SchemaError::FrameExceeded { length });
    }
    out.extend_from_slice(&plain[..4 + prefix]);
    out[0..4].copy_from_slice(&(length as u32).to_le_bytes());
    out[4 + 5] |= flag::COMPRESSED;
    out[4 + 6] = compress::ALGO_LZ4_DICT;
    out.extend_from_slice(&blob);
    Ok(())
}

/// Encode com o dicionário TREINADO (v1.3 §4.8): mesmas regras dos demais
/// (threshold, nunca inflar, teto), com o algoritmo `ALGO_ZSTD_DICT` no byte
/// reservado. O dicionário é treinado sobre os nomes canônicos do registro do
/// servidor em AMBOS os lados ([`compress::zstd_dict_from_registry`]) — só
/// quem negociou `caps::DICT + caps::ZSTD` e já completou o `HELLO` consegue
/// decodificar; os demais falham fechado.
pub fn encode_with_zstd_dict(
    msg: &Message,
    dict: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), SchemaError> {
    let mut plain = Vec::with_capacity(HEADER_LEN + msg.name.len() + 64);
    encode(msg, &mut plain)?;
    let ts_len = usize::from(msg.timestamp_us.is_some());
    let prefix = HEADER_LEN + 8 * ts_len;
    let region = &plain[4 + prefix..];
    if region.len() <= compress::THRESHOLD {
        out.extend_from_slice(&plain);
        return Ok(());
    }
    // Teto do compressBound zstd (src + src/255 + 16): blob nunca estoura o
    // buffer por conta da API; o "nunca inflar" segue valendo.
    let mut blob = vec![0u8; region.len() + region.len() / 255 + 16];
    let n = zstd::bulk::Compressor::with_dictionary(compress::ZSTD_LEVEL, dict)
        .and_then(|mut c| c.compress_to_buffer(region, &mut blob))
        .map_err(|_| SchemaError::CompressionFailed)?;
    blob.truncate(n);
    if blob.len() >= region.len() {
        out.extend_from_slice(&plain);
        return Ok(());
    }
    let length = prefix + blob.len();
    if length > MAX_PAYLOAD {
        return Err(SchemaError::FrameExceeded { length });
    }
    out.extend_from_slice(&plain[..4 + prefix]);
    out[0..4].copy_from_slice(&(length as u32).to_le_bytes());
    out[4 + 5] |= flag::COMPRESSED;
    out[4 + 6] = compress::ALGO_ZSTD_DICT;
    out.extend_from_slice(&blob);
    Ok(())
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
    decode_with_dict(buf, None)
}

/// Decode com o dicionário LZ4 da conexão (v1.2 §4.8): `dict = None` é o
/// comportamento v1.1 exato (id 2 ⇒ `UnknownCompression`, princípio 7).
/// Desbloqueia SOMENTE o id 2 — o id 3 (zstd treinado, v1.3) sem o
/// dicionário tipado certo é `UnknownCompression{3}`, exatamente o que um
/// codec v1.2 faz (fail closed).
pub fn decode_with_dict(buf: &[u8], dict: Option<&[u8]>) -> Result<(Message, usize), SchemaError> {
    let dict = dict.map(|d| compress::DictConexao::Lz4(d.to_vec()));
    decode_com(buf, dict.as_ref())
}

/// Decode com o dicionário TIPADO da conexão (v1.2/v1.3 §4.8): id 2 exige a
/// matéria concatenada ([`compress::DictConexao::Lz4`]); id 3 exige a
/// treinada ([`compress::DictConexao::Zstd`]) — o contrário é
/// `UnknownCompression` (fail closed por construção).
pub fn decode_with_conexao(
    buf: &[u8],
    dict: Option<&compress::DictConexao>,
) -> Result<(Message, usize), SchemaError> {
    decode_com(buf, dict)
}

fn decode_com(
    buf: &[u8],
    dict: Option<&compress::DictConexao>,
) -> Result<(Message, usize), SchemaError> {
    let total = peek_frame_len(buf)?;
    if total == 0 || buf.len() < total {
        return Err(SchemaError::MissingField);
    }
    let payload = &buf[4..total];
    if payload.len() < HEADER_LEN {
        return Err(SchemaError::PayloadTooShort {
            length: payload.len(),
        });
    }
    if payload[0..3] != MAGIC {
        return Err(SchemaError::InvalidMagic);
    }
    if payload[3] != VERSION {
        return Err(SchemaError::UnknownVersion {
            received: payload[3],
        });
    }
    let opcode = payload[4];
    let flags = payload[5];
    let reservado = payload[6];
    let name_len = payload[7] as usize;
    let seq = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
    if op::name(opcode).is_none() {
        return Err(SchemaError::UnknownOpcode { received: opcode });
    }

    // Região nome+corpo: plana ou comprimida (§4.8). O timestamp (§5), quando
    // presente, viaja plano logo após o header.
    let mut region_owned;
    let mut rest: &[u8] = &payload[HEADER_LEN..];
    let mut timestamp_us: Option<u64> = None;
    if flags & flag::TIMESTAMP != 0 {
        if rest.len() < 8 {
            return Err(SchemaError::MissingField);
        }
        timestamp_us = Some(u64::from_le_bytes([
            rest[0], rest[1], rest[2], rest[3], rest[4], rest[5], rest[6], rest[7],
        ]));
        rest = &rest[8..];
    }
    if flags & flag::COMPRESSED != 0 {
        // §4.8/v1.2/v1.3: algoritmo no byte reservado; guarda de bomba = teto
        // da região descomprimida (blob corrupto/grande demais ⇒ erro).
        match reservado {
            compress::ALGO_LZ4 => {
                // Buffer-teto = guarda de bomba: blob que exceder 8192
                // descomprimido falha; o que sobra é truncado após o fato.
                region_owned = vec![0u8; MAX_PAYLOAD];
                let n = lz4_flex::block::decompress_into(rest, &mut region_owned)
                    .map_err(|_| SchemaError::DecompressionFailed)?;
                region_owned.truncate(n);
                rest = &region_owned;
            }
            compress::ALGO_LZ4_DICT => {
                // v1.2: sem o dicionário LZ4 da conexão, id 2 é desconhecido —
                // exatamente o que um codec v1.1 faz (fail closed).
                let Some(compress::DictConexao::Lz4(dict)) = dict else {
                    return Err(SchemaError::UnknownCompression {
                        received: reservado,
                    });
                };
                region_owned = vec![0u8; MAX_PAYLOAD];
                let n = lz4_flex::block::decompress_into_with_dict(rest, &mut region_owned, dict)
                    .map_err(|_| SchemaError::DecompressionFailed)?;
                region_owned.truncate(n);
                rest = &region_owned;
            }
            compress::ALGO_ZSTD_DICT => {
                // v1.3: id 3 exige o dicionário TREINADO — com a matéria do
                // id 2 (ou sem dict) é desconhecido (fail closed).
                let Some(compress::DictConexao::Zstd(dict)) = dict else {
                    return Err(SchemaError::UnknownCompression {
                        received: reservado,
                    });
                };
                region_owned = vec![0u8; MAX_PAYLOAD];
                let n = zstd::bulk::Decompressor::with_dictionary(dict)
                    .and_then(|mut d| d.decompress_to_buffer(rest, &mut region_owned))
                    .map_err(|_| SchemaError::DecompressionFailed)?;
                region_owned.truncate(n);
                rest = &region_owned;
            }
            _ => {
                return Err(SchemaError::UnknownCompression {
                    received: reservado,
                })
            }
        }
    }

    let mut r = Reader::new(rest);
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
                4 => AckAct::FallbackExecuted {
                    alternativo: r.len_string(1, MAX_NAME)?,
                },
                5 => AckAct::FallbackExhausted,
                6 => AckAct::InvalidValue {
                    reason: r.len_string(1, MAX_STRING)?,
                },
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
                        DeviceDesc::Sensor {
                            name,
                            min,
                            max,
                            quantity,
                            unit,
                            precision_pct,
                        }
                    }
                    1 => {
                        let flags = r.u8()?;
                        let min = r.opt_f64(flags, 1)?;
                        let max = r.opt_f64(flags, 2)?;
                        let safety = r.opt_f64(flags, 4)?;
                        DeviceDesc::Actor {
                            name,
                            min,
                            max,
                            safety,
                        }
                    }
                    _ => return Err(SchemaError::MissingField),
                });
            }
            r.end()?;
            Body::Hello { devices }
        }
        op::CAPS | op::CAPS_OK => {
            let capabilities = r.u16()?;
            r.end()?;
            // Bits reservados de capacidades são ignorados no decode (§4.5).
            Body::Caps { capabilities }
        }
        op::READ_BATCH => {
            let count = r.u16()? as usize;
            if count == 0 || count > MAX_BATCH {
                return Err(SchemaError::BatchTooLarge);
            }
            let mut names = Vec::with_capacity(count);
            for _ in 0..count {
                names.push(r.len_string(1, MAX_NAME)?);
            }
            r.end()?;
            Body::ReadBatch { names }
        }
        op::READ_BATCH_OK => {
            let count = r.u16()? as usize;
            if count == 0 || count > MAX_BATCH {
                return Err(SchemaError::BatchTooLarge);
            }
            let mut results = Vec::with_capacity(count);
            for _ in 0..count {
                results.push(match r.u8()? {
                    0 => BatchResult::Ok {
                        value: r.f64()?,
                        canonical: r.len_string(1, MAX_NAME)?,
                    },
                    s @ 1..=3 => BatchResult::Err { reason: s },
                    // 4 = nao_registrado (§4.1 valor 0; o byte 0 pertence a Ok).
                    4 => BatchResult::Err {
                        reason: reason::NOT_REGISTERED,
                    },
                    _ => return Err(SchemaError::MissingField),
                });
            }
            r.end()?;
            Body::ReadBatchOk { results }
        }
        op::AUTH_CHALLENGE => {
            let scheme = r.u16()?;
            if scheme != AUTH_SCHEME_PSK_HMAC_SHA256 {
                return Err(SchemaError::UnknownAuthScheme { received: scheme });
            }
            let mut nonce = [0u8; AUTH_NONCE_LEN];
            nonce.copy_from_slice(r.take(AUTH_NONCE_LEN)?);
            r.end()?;
            Body::AuthChallenge { scheme, nonce }
        }
        op::AUTH_RESPONSE => {
            let mut nonce = [0u8; AUTH_NONCE_LEN];
            nonce.copy_from_slice(r.take(AUTH_NONCE_LEN)?);
            let mut mac = [0u8; AUTH_NONCE_LEN];
            mac.copy_from_slice(r.take(AUTH_NONCE_LEN)?);
            r.end()?;
            Body::AuthResponse { nonce, mac }
        }
        // Corpos vazios por último (READ exige nome; HEARTBEAT/BYE/AUTH_OK não).
        op::READ | op::HEARTBEAT | op::BYE | op::AUTH_OK => {
            if matches!(opcode, op::READ) && name.is_empty() {
                return Err(SchemaError::InvalidName);
            }
            r.end()?;
            Body::Empty
        }
        _ => unreachable!("opcode validado acima"),
    };

    // Bits de recurso (TIMESTAMP/COMPRESSED) são derivados no encode — a
    // mensagem decodificada carrega a semântica (`timestamp_us`), não a flag;
    // assim `decode(encode(m)) == m` vale também para frames carimbados.
    let semantic_flags = flags & !(flag::TIMESTAMP | flag::COMPRESSED);
    Ok((
        Message {
            opcode,
            flags: semantic_flags,
            seq,
            name,
            timestamp_us,
            body,
        },
        total,
    ))
}
