//! vbl-fxp — Flux Protocol da VerboLang (Etapa 3, PLAN §3).
//!
//! Camada única de I/O que unifica sensores (entrada) e atores (saída)
//! (FORMAL §4.4/§6), consumida pelo runtime via trait `vbl_runtime::Fxp`.
//!
//! Módulos:
//! - [`schema`]: codec do schema de mensagem v1 (docs/FXP-SCHEMA-v1.md —
//!   definido antes dos drivers; serialização sem perda, LE, ack/seq);
//! - [`registry`]: registro de dispositivos com aliases (§6), modos
//!   real/simulado/híbrido e política de fallback (§4.3);
//! - [`drivers`]: backends reais (sysfs/thermal_zone, RAPL, hwmon PWM, LED)
//!   e o `AttentionSource` (simulado obrigatório em CI);
//! - [`queue`]: fila de comandos com prioridade (subvert = máxima), timeout
//!   e retry (PLAN §3.4);
//! - [`transport`]: frames v1 sobre in-process/Unix/TCP com ack/timeout;
//! - [`bus`]: o barramento `FxpBus` (trait `Fxp` do runtime) que roteia
//!   leituras/atos por modo de operação com honestidade de dados (§4.7).
//!
//! O simulador determinístico continua em `vbl-runtime::sim` (contrato da
//! Etapa 1/2); o bus o usa como backend simulado — modo `simulado` é bit a bit
//! compatível com a Etapa 2.

pub mod auth;
pub mod bus;
pub mod discover;
pub mod drivers;
/// mDNS/DNS-SD (v1.2 §4.10) — opt-in: compile com `--features mdns`.
#[cfg(feature = "mdns")]
pub mod mdns;
pub mod peer;
pub mod queue;
pub mod registry;
pub mod schema;
pub mod tls;
pub mod transport;

pub use auth::{
    mac as auth_mac, nonce as auth_nonce, verify as auth_verify, NONCE_LEN as AUTH_NONCE_BYTES,
};
pub use bus::{BusConfig, FxpBus, Route};
pub use discover::{
    decode_beacon, discover_peers, encode_beacon, registry_hash, Announcer, Beacon, DiscoveredPeer,
    DiscoveryError, DEFAULT_GROUP, DEFAULT_INTERVAL, MULTICAST_TTL,
};
pub use drivers::{ActorDriver, AttentionSource, SensorDriver, SimulatedAttention};
pub use peer::{PeerConfig, PeerServer};
pub use queue::{Command, CommandQueue, QueueError, PRIORITY_NORMAL, PRIORITY_SUBVERT};
pub use registry::{
    DeviceEntry, DeviceKind, DeviceMode, DeviceRegistry, Endpoint, FxpConfig, OperationMode,
    RegistryError, RemoteAddr,
};
pub use schema::{AckAct, BatchResult, Body, DeviceDesc, Message, SchemaError, WireValue};
pub use tls::{ConfiancaCliente, TlsAccept, TofuFalha, TofuStore, Trust};
pub use transport::{Server, TransportError};
