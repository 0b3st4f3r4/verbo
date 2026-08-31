//! vbl-runtime — motor de tick da VerboLang (Etapa 2, PLAN §2.2).
//!
//! Componentes (FORMAL §4):
//! - [`caderno`]: log termodinâmico com cadeia SHA-256;
//! - [`fxp`]/[`sim`]: barramento de I/O (trait) + simulador determinístico;
//! - [`forma`]: forma ativa (horizonte absoluto, manutenção, retenção);
//! - [`scheduler`]: fila de prazos (min-heap por horizon/maintenance);
//! - [`engine`]: loop de tick — regras na ordem declarada, prazos depois;
//! - [`main_interp`]: bloco `main` (keep/act/every);
//! - [`loader`]: AST → runtime + validação contra o registro do FXP;
//! - [`persist`]: `equilibrium` em suporte estável (.vl canônico + SHA-256);
//! - [`json`]: serialização JSON determinística (auditoria do Caderno).

pub mod caderno;
pub mod caderno_producao;
pub mod engine;
pub mod forma;
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
static AUDITOR_DE_HEAP: heap_auditor::AuditorAlloc = heap_auditor::AuditorAlloc;

pub use caderno::{Caderno, ChainCaderno, Evento};
pub use caderno_producao::{
    jsonl_de_binario, verificar, verificar_binario, verificar_jsonl, CadernoProducao, RelatorioVerificacao,
    Resumo,
};
pub use engine::Engine;
pub use forma::{ActionRt, Form, Manutencao, RuleRt, VALOR_POETICO_CANONICO};
pub use fxp::{ActOutcome, ActorLimits, FalhaSensor, Fxp, Limite, Registry, SensorInfo, Value};
pub use loader::{carregar, validar};
pub use main_interp::{MainInterpreter, StmtRt};
pub use sim::FxpSimulator;
