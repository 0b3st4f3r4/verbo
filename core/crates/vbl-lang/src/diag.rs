//! Diagnósticos de compilação — código canônico + posição (linha/coluna).
//!
//! Os códigos reancoram a suíte da Etapa 1 (`tests/vlcheck.py` e
//! `tests/fxp_sim/contract.py`): cada cláusula de erro da FORMAL tem um
//! código estável que a matriz de rastreabilidade da Etapa 2 referencia
//! (`docs/STAGE-2-TRACEABILITY-MATRIX.md`).

/// Posição no fonte (linha e coluna, ambas 1-based, contadas em caracteres).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(line: u32, col: u32) -> Self {
        Self { line, col }
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// Gravidade do diagnóstico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Erro de compilação — o programa não carrega.
    Error,
    /// Aviso — o programa carrega (uso reservado).
    Warning,
}

/// Um diagnóstico: `código line:coluna mensagem` (critério do AGENTS.md §1.3:
/// "mensagens de erro claras, indicando linha e coluna").
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub span: Span,
    pub severity: Severity,
}

impl Diagnostic {
    pub fn error(code: &str, span: Span, message: impl Into<String>) -> Self {
        Self { code: code.to_owned(), message: message.into(), span, severity: Severity::Error }
    }

    pub fn warning(code: &str, span: Span, message: impl Into<String>) -> Self {
        Self { code: code.to_owned(), message: message.into(), span, severity: Severity::Warning }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let severity = match self.severity {
            Severity::Error => "erro",
            Severity::Warning => "aviso",
        };
        write!(f, "{} [{}] {}: {}", self.span, severity, self.code, self.message)
    }
}

/// Coleção de diagnósticos com atalhos de consulta (espelha `vlcheck.validate`).
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.items.push(Diagnostic::error(code, span, message));
    }

    pub fn warning(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.items.push(Diagnostic::warning(code, span, message));
    }

    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter().filter(|d| d.is_error())
    }

    /// Ordena por posição (estável) — mesmo contrato de `vlcheck.validate`.
    pub fn sort(&mut self) {
        self.items.sort_by_key(|d| (d.span.line, d.span.col));
    }

    pub fn codes(&self) -> std::collections::BTreeSet<String> {
        self.items.iter().map(|d| d.code.clone()).collect()
    }

    pub fn contains(&self, code: &str) -> bool {
        self.items.iter().any(|d| d.code == code)
    }
}

impl std::fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for d in &self.items {
            writeln!(f, "{d}")?;
        }
        Ok(())
    }
}
