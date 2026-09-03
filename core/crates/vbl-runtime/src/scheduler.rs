//! Escalonador por fila de prazos (FORMAL §4.2): min-heap por
//! `horizon`/`maintenance_deadline` — O(log N) por mutação e varredura
//! O(N + vencidos) por tick. O relógio é virtual e injetável: o engine passa
//! `now` a cada tick (1 tick ≈ 1 s virtual; em teste o simulador avança
//! instantaneamente).
//!
//! O heap decide **quem venceu**; a validade do prazo (predicados pinados na
//! Etapa 1: `horizon` com `>=`, manutenção com `>` estrito, versões de
//! `keep`) é conferida pelo engine, que pode reagendar a entrada para o
//! próximo tick — o heap nunca perde um prazo.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Tipo de prazo agendado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deadline {
    Horizon,
    Maintenance,
}

/// Instante com ordenação total (`f64::total_cmp`) — heap exige `Ord`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualInstant(f64);

impl VirtualInstant {
    pub fn de(x: f64) -> Self {
        Self(x)
    }

    pub fn value(&self) -> f64 {
        self.0
    }
}

impl PartialOrd for VirtualInstant {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VirtualInstant {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Eq for VirtualInstant {}

/// Entrada da fila: (instante-alvo, forma, tipo, versão do estado do prazo).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub em: VirtualInstant,
    pub form: String,
    pub deadline: Deadline,
    /// Versão do estado que gerou a entrada (`keep` renova a versão —
    /// entradas obsoletas são descartadas pelo engine).
    pub version: u64,
    /// Ordem de inserção (desempate FIFO no heap).
    pub seq: u64,
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // ordem natural: por instante (o heap usa Reverse<Entrada> → min-heap);
        // empate: ordem de inserção (seq — FIFO)
        (self.em, self.seq).cmp(&(other.em, other.seq))
    }
}

/// Min-heap de prazos de formas ativas.
#[derive(Debug, Default)]
pub struct Scheduler {
    heap: BinaryHeap<Reverse<Entry>>,
    sequence: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Agenda um prazo — O(log N).
    ///
    /// `version` é a versão do estado da forma que gerou o prazo (`keep`
    /// renova a versão); entradas obsoletas são descartadas pelo engine.
    pub fn schedule(&mut self, form: &str, deadline: Deadline, em: f64, version: u64) {
        self.sequence += 1;
        let entry = Entry {
            em: VirtualInstant::de(em),
            form: form.into(),
            deadline,
            version,
            seq: self.sequence,
        };
        self.heap.push(Reverse(entry));
    }

    /// Drena os prazos vencidos (`em <= now`) — O(log N + vencidos).
    pub fn drain_due(&mut self, now: f64) -> Vec<Entry> {
        let now = VirtualInstant::de(now);
        let mut due = Vec::new();
        while let Some(Reverse(top)) = self.heap.peek() {
            if top.em > now {
                break;
            }
            let Reverse(entry) = self.heap.pop().unwrap();
            due.push(entry);
        }
        due
    }

    /// Remove todas as entradas de uma forma (dissolução explícita).
    pub fn remove_form(&mut self, form: &str) {
        let remaining: Vec<Reverse<Entry>> = self
            .heap
            .drain()
            .filter(|Reverse(e)| e.form != form)
            .collect();
        self.heap = remaining.into_iter().collect();
    }

    /// Próximo prazo agendado (telemetria/CLI).
    pub fn next(&self) -> Option<&Entry> {
        self.heap.peek().map(|Reverse(e)| e)
    }
}
