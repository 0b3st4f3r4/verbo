//! vbl-runtime — motor de tick da VerboLang (Etapa 2, PLAN §2.2).
//!
//! Componentes (FORMAL §4):
//! - [`ledger`]: log termodinâmico com cadeia SHA-256;
//! - [`fxp`]/[`sim`]: barramento de I/O (trait) + simulador determinístico;
//! - [`form`]: forma ativa (horizonte absoluto, manutenção, retenção);
//! - [`scheduler`]: fila de prazos (min-heap por horizon/maintenance);
//! - [`engine`]: loop de tick — regras na ordem declarada, prazos depois;
//! - [`main_interp`]: bloco `main` (keep/act/every);
//! - [`loader`]: AST → runtime + validação contra o registro do FXP;
//! - [`persist`]: `equilibrium` em suporte estável (.vl canônico + SHA-256);
//! - [`json`]: serialização JSON determinística (auditoria do Caderno).

pub mod ledger;
pub mod production_ledger;
pub mod engine;
pub mod form;
pub mod fxp;
#[cfg(feature = "heap-audit")]
pub mod heap_auditor;
pub mod json;
pub mod loader;
pub mod main_interp;
pub mod persist;
pub mod scheduler;
pub mod sim;

/// Alocador global de contagem — SOMENTE com a feature `heap-audit` (Etapa 5:
/// fechamento físico dos orçamentos de retenção; builds de produção não pagam
/// nada por isso).
#[cfg(feature = "heap-audit")]
#[global_allocator]
static HEAP_AUDITOR: heap_auditor::AuditorAlloc = heap_auditor::AuditorAlloc;

pub use ledger::{Ledger, ChainLedger, LedgerEvent};
pub use production_ledger::{
    jsonl_from_binary, verify, verify_binary, verify_jsonl, ProductionLedger, VerificationReport,
    Summary,
};
pub use engine::Engine;
pub use form::{ActionRt, Form, Maintenance, RuleRt, CANONICAL_POETIC_VALUE};
pub use fxp::{ActOutcome, ActorLimits, SensorFailure, Fxp, Limit, Registry, SensorInfo, Value};
pub use loader::{load, validate};
pub use main_interp::{MainInterpreter, StmtRt};
pub use sim::FxpSimulator;
