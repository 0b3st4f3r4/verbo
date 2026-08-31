//! AST do programa `.vl` (FORMAL §3).
//!
//! A AST preserva os metadados exigidos pelo AGENTS.md §1.3 (horizon,
//! source_path, maintenance_deadline etc.) e valida a aplicabilidade dos
//! opcionais por conjugação. `value` é opaco ao runtime (FORMAL §3,
//! nota sobre `expression`).

use crate::diag::Span;

/// Conjugações (FORMAL §3/§4.1): `event`, `equilibrium`, `nonequilibrium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conjugation {
    Event,
    Equilibrium,
    Nonequilibrium,
}

impl Conjugation {
    pub fn nome(&self) -> &'static str {
        match self {
            Conjugation::Event => "event",
            Conjugation::Equilibrium => "equilibrium",
            Conjugation::Nonequilibrium => "nonequilibrium",
        }
    }

    /// `currency` padrão da conjugação (FORMAL §3, nota sobre `currency`).
    pub fn currency_padrao(&self) -> &'static str {
        match self {
            Conjugation::Event => "CpuCycles",
            Conjugation::Equilibrium => "DiskBytes",
            Conjugation::Nonequilibrium => "PowerWatts",
        }
    }
}

/// Unidades de tempo (FORMAL §3: `duration = number time_unit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    S,
    Ms,
    Us,
    Ns,
}

impl TimeUnit {
    pub fn fator(&self) -> f64 {
        match self {
            TimeUnit::S => 1.0,
            TimeUnit::Ms => 1e-3,
            TimeUnit::Us => 1e-6,
            TimeUnit::Ns => 1e-9,
        }
    }

    pub fn sufixo(&self) -> &'static str {
        match self {
            TimeUnit::S => "s",
            TimeUnit::Ms => "ms",
            TimeUnit::Us => "us",
            TimeUnit::Ns => "ns",
        }
    }
}

/// Duração: número + unidade, convertida para segundos (f64).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duration {
    pub valor: f64,
    pub unit: TimeUnit,
    pub span: Span,
}

impl Duration {
    pub fn segundos(&self) -> f64 {
        self.valor * self.unit.fator()
    }
}

/// Unidades físicas de threshold (FORMAL §3: `physical_unit = 'W' | '°C'`;
/// `%` vem de `percentage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalUnit {
    W,
    DegC,
    Percent,
}

impl PhysicalUnit {
    pub fn simbolo(&self) -> &'static str {
        match self {
            PhysicalUnit::W => "W",
            PhysicalUnit::DegC => "°C",
            PhysicalUnit::Percent => "%",
        }
    }

    /// Grandeza canônica associada (FORMAL §6 — registro mínimo).
    pub fn grandeza(&self) -> &'static str {
        match self {
            PhysicalUnit::W => "potencia",
            PhysicalUnit::DegC => "temperatura",
            PhysicalUnit::Percent => "atencao",
        }
    }
}

/// `expression = string | number | identifier` — conteúdo lógico opaco
/// (FORMAL §3, nota: "o campo `value` não é interpretado pelo runtime").
#[derive(Debug, Clone, PartialEq)]
pub struct Expression {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Str(String),
    Num(f64),
    Ident(String),
}

impl Expression {
    pub fn str(s: impl Into<String>, span: Span) -> Self {
        Self { kind: ExprKind::Str(s.into()), span }
    }

    pub fn num(v: f64, span: Span) -> Self {
        Self { kind: ExprKind::Num(v), span }
    }

    pub fn ident(nome: impl Into<String>, span: Span) -> Self {
        Self { kind: ExprKind::Ident(nome.into()), span }
    }
}

/// Threshold de regra: número puro ou com unidade (convertido para valor
/// numérico antes da comparação — FORMAL §3, nota sobre `threshold`).
#[derive(Debug, Clone, PartialEq)]
pub struct Threshold {
    pub valor: f64,
    /// `None` = número puro (sem unidade declarada).
    pub unit: Option<PhysicalUnit>,
}

/// `sensor_ref = identifier | string` — nome simbólico no FXP.
#[derive(Debug, Clone, PartialEq)]
pub struct SensorRef {
    pub nome: String,
    pub span: Span,
}

/// Operadores de comparação (FORMAL §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

impl CmpOp {
    pub fn simbolo(&self) -> &'static str {
        match self {
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
        }
    }

    pub fn avalia(&self, sensor: f64, limiar: f64) -> bool {
        match self {
            CmpOp::Lt => sensor < limiar,
            CmpOp::Gt => sensor > limiar,
            CmpOp::Le => sensor <= limiar,
            CmpOp::Ge => sensor >= limiar,
            CmpOp::Eq => sensor == limiar,
            CmpOp::Ne => sensor != limiar,
        }
    }
}

/// Ações da `action_list` (FORMAL §3).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Dissolve,
    Subvert,
    ReclassifyAsEquilibrium,
    ReclassifyAsNonequilibrium,
    NotifyShutdown,
    /// `act '(' actor_name ',' expression ')'` — comando FXP ao ator.
    Act { actor: String, value: Expression, actor_span: Span },
}

/// Regra de revisão: `when sensor_ref op threshold '->' action_list`.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub sensor: SensorRef,
    pub op: CmpOp,
    pub threshold: Threshold,
    pub actions: Vec<Action>,
    pub span: Span,
}

/// Atributos opcionais de forma (além de `value`/`horizon` obrigatórios).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormAttrs {
    pub source_path: Option<String>,
    /// apenas `nonequilibrium` — obrigatório nela (FORMAL §3).
    pub maintenance_deadline: Option<Duration>,
    /// apenas `nonequilibrium` — `"cooperation" | "extraction"` (canônicos).
    pub exchange_mode: Option<String>,
    /// apenas `equilibrium`.
    pub cost_bytes: Option<i128>,
    pub currency: Option<String>,
    pub classification: Option<String>,
}

/// Declaração de forma: `conjugation_kw identifier '{' form_body '}'`.
#[derive(Debug, Clone, PartialEq)]
pub struct FormDecl {
    pub conjugation: Conjugation,
    pub name: String,
    /// `value` — obrigatório, primeiro atributo (Lei 1 / FORMAL §3).
    pub value: Expression,
    /// `horizon` — obrigatório, segundo atributo; ABSOLUTO (FORMAL §4.1).
    pub horizon: Duration,
    pub attrs: FormAttrs,
    pub span: Span,
}

/// Declaração de revisão: `review identifier '{' regras '}'`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewDecl {
    pub form: String,
    pub rules: Vec<Rule>,
    pub span: Span,
}

/// Statements do bloco `main` (FORMAL §3).
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Keep(String),
    Act { actor: String, value: Expression },
    Every { period: Duration, body: Vec<Statement> },
}

/// Bloco `main` — opcional, uma única vez, após as demais declarações.
#[derive(Debug, Clone, PartialEq)]
pub struct MainBlock {
    pub statements: Vec<Statement>,
    pub span: Span,
}

/// Declarações de topo, em ordem de declaração (a ordem importa: reviews se
/// ligam a formas já declaradas; `main` fecha o programa).
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub decls: Vec<Declaration>,
    pub main: Option<MainBlock>,
}

#[derive(Debug, Clone)]
pub enum Declaration {
    Form(Box<FormDecl>),
    Review(ReviewDecl),
}

impl Program {
    pub fn forms(&self) -> impl Iterator<Item = &FormDecl> {
        self.decls.iter().filter_map(|d| match d {
            Declaration::Form(f) => Some(f.as_ref()),
            _ => None,
        })
    }

    pub fn reviews(&self) -> impl Iterator<Item = &ReviewDecl> {
        self.decls.iter().filter_map(|d| match d {
            Declaration::Review(r) => Some(r),
            _ => None,
        })
    }
}
