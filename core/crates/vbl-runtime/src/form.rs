//! Forma ativa no runtime (FORMAL §4.1).
//!
//! Abstracionismo de tipos: nenhuma variável inerte — toda estrutura tem
//! `horizon` explícito (ABSOLUTO, contado da criação; reclassificações não o
//! renovam — Lei 1). O `value` é conteúdo lógico opaco ao motor.
//!
//! Orçamentos de retenção (ADR-001): os contadores do runtime são PROXY
//! determinístico de heap (livro-razão por forma); a medição física
//! (`size_of` + arenas, miri/ASan) fecha na Etapa 5 (PLAN §5.1).

use crate::fxp::Value;
use vbl_lang::Conjugation;

/// Valor poético canônico da subversão (FORMAL §4.5).
pub const VALOR_POETICO_CANONICO: &str =
    "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente";

/// Orçamentos de retenção por conjugação em bytes (ADR-001).
pub const ORCAMENTO_RETENCAO: (u64, u64, u64) = (256, 1024, 512); // event, equilibrium, nonequilibrium

/// Ações compiladas para o runtime (threshold já convertido para número puro).
#[derive(Debug, Clone, PartialEq)]
pub enum ActionRt {
    Dissolve,
    Subvert,
    ReclassifyEquilibrium,
    ReclassifyNonequilibrium,
    NotifyShutdown,
    Act { ator: String, valor: Value },
}

/// Regra de revisão compilada.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleRt {
    pub sensor: String,
    pub op: vbl_lang::CmpOp,
    pub threshold: f64,
    pub actions: Vec<ActionRt>,
}

/// Conjugação com estado de manutenção (`nonequilibrium` apenas).
#[derive(Debug, Clone, PartialEq)]
pub struct Manutencao {
    /// Prazo entre manutenções (s) — o DECLARADO sobrevive a reclassificações.
    pub deadline_s: f64,
    /// Instante virtual da última manutenção (keep implícito ou explícito).
    pub ultima: f64,
}

/// Forma ativa.
#[derive(Debug, Clone)]
pub struct Form {
    pub name: String,
    pub value: Value,
    /// Horizon em segundos — ABSOLUTO desde `creation_time` (FORMAL §4.1).
    pub horizon_s: f64,
    /// Instante virtual de criação; preservado nas reclassificações.
    pub creation_time: f64,
    pub conjugation: Conjugation,
    /// `currency` de contabilização no Caderno (padrão da conjugação).
    pub currency: String,
    /// Nome simbólico do sensor principal (FORMAL §3), se declarado.
    pub source_path: Option<String>,
    /// Anotação de auditoria (sem efeito semântico — FORMAL §3).
    pub classification: Option<String>,
    /// Prazo declarado alguma vez pela forma (habilita NEQ→EQ→NEQ — FORMAL §3).
    pub declared_maintenance_deadline: Option<f64>,
    /// Estado de manutenção; `Some` apenas em `nonequilibrium`.
    pub manutencao: Option<Manutencao>,
    /// Anotação de auditoria `cooperation`/`extraction` (PLAN §2.2).
    pub exchange_mode: Option<String>,
    /// Custo em bytes (`equilibrium`); `None` → tamanho real gravado (§4.1).
    pub cost_bytes: Option<u64>,
    /// Regras de revisão ativas (sobrevivem à reclassificação — Etapa 1).
    pub rules: Vec<RuleRt>,
    pub dissolvida: bool,
    /// Versão do estado do horizon (muda na (re)classificação) — o
    /// escalonador descarta entradas obsoletas pela versão.
    pub horizon_versao: u64,
    /// Versão do estado de manutenção (`keep` renova; 0 fora de NEQ).
    pub manutencao_versao: u64,
}

impl Form {
    /// Cabe em que instante a forma expira (criação + horizon).
    pub fn horizonte_fim(&self) -> f64 {
        self.creation_time + self.horizon_s
    }

    /// `horizon` esgotado? (`>=` — no limite exato expira; Etapa 1, FORMAL §4.1)
    pub fn horizon_esgotado(&self, agora: f64) -> bool {
        (agora - self.creation_time) >= self.horizon_s
    }

    /// Prazo de manutenção vencido? (`>` estritamente maior — Etapa 1)
    pub fn manutencao_vencida(&self, agora: f64) -> bool {
        match &self.manutencao {
            Some(m) => (agora - m.ultima) > m.deadline_s,
            None => false,
        }
    }

    /// Manutenção: renova o prazo (keep implícito ou explícito).
    pub fn keep(&mut self, agora: f64) {
        if let Some(m) = &mut self.manutencao {
            m.ultima = agora;
        }
    }

    /// Contador de retenção do runtime (proxy de heap — ADR-001).
    pub fn bytes_retidos(&self) -> u64 {
        let base: u64 = match self.conjugation {
            Conjugation::Event => 96,
            Conjugation::Equilibrium => 128,
            Conjugation::Nonequilibrium => 160,
        };
        let valor = match &self.value {
            Value::Num(n) => n.to_string().len(),
            Value::Str(s) | Value::Ident(s) => s.len(),
        } as u64;
        base + valor + 32 * self.rules.len() as u64
    }
}

/// Registro de livros do runtime (contadores de retenção por forma).
#[derive(Debug, Clone, Default)]
pub struct Retencao {
    pub por_forma: std::collections::BTreeMap<String, u64>,
    /// Estruturas de trabalho laborativo (NEQ): prazo + último keep.
    pub labor: std::collections::BTreeMap<String, u64>,
}
