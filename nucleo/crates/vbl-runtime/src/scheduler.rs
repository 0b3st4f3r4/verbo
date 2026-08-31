//! Escalonador por fila de prazos (FORMAL §4.2): min-heap por
//! `horizon`/`maintenance_deadline` — O(log N) por mutação e varredura
//! O(N + vencidos) por tick. O relógio é virtual e injetável: o engine passa
//! `agora` a cada tick (1 tick ≈ 1 s virtual; em teste o simulador avança
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
pub enum Prazo {
    Horizon,
    Manutencao,
}

/// Instante com ordenação total (`f64::total_cmp`) — heap exige `Ord`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instante(f64);

impl Instante {
    pub fn de(x: f64) -> Self {
        Self(x)
    }

    pub fn valor(&self) -> f64 {
        self.0
    }
}

impl PartialOrd for Instante {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Instante {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Eq for Instante {}

/// Entrada da fila: (instante-alvo, forma, tipo, versão do estado do prazo).
#[derive(Debug, Clone, PartialEq)]
pub struct Entrada {
    pub em: Instante,
    pub forma: String,
    pub prazo: Prazo,
    /// Versão do estado que gerou a entrada (`keep` renova a versão —
    /// entradas obsoletas são descartadas pelo engine).
    pub versao: u64,
    /// Ordem de inserção (desempate FIFO no heap).
    pub seq: u64,
}

impl Eq for Entrada {}

impl PartialOrd for Entrada {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entrada {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // ordem natural: por instante (o heap usa Reverse<Entrada> → min-heap);
        // empate: ordem de inserção (seq — FIFO)
        (self.em, self.seq).cmp(&(other.em, other.seq))
    }
}

/// Min-heap de prazos de formas ativas.
#[derive(Debug, Default)]
pub struct Scheduler {
    heap: BinaryHeap<Reverse<Entrada>>,
    sequencia: u64,
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
    /// `versao` é a versão do estado da forma que gerou o prazo (`keep`
    /// renova a versão); entradas obsoletas são descartadas pelo engine.
    pub fn agendar(&mut self, forma: &str, prazo: Prazo, em: f64, versao: u64) {
        self.sequencia += 1;
        let entrada =
            Entrada { em: Instante::de(em), forma: forma.into(), prazo, versao, seq: self.sequencia };
        self.heap.push(Reverse(entrada));
    }

    /// Drena os prazos vencidos (`em <= agora`) — O(log N + vencidos).
    pub fn drenar_vencidos(&mut self, agora: f64) -> Vec<Entrada> {
        let agora = Instante::de(agora);
        let mut vencidos = Vec::new();
        while let Some(Reverse(topo)) = self.heap.peek() {
            if topo.em > agora {
                break;
            }
            let Reverse(entrada) = self.heap.pop().unwrap();
            vencidos.push(entrada);
        }
        vencidos
    }

    /// Remove todas as entradas de uma forma (dissolução explícita).
    pub fn remover_forma(&mut self, forma: &str) {
        let restantes: Vec<Reverse<Entrada>> =
            self.heap.drain().filter(|Reverse(e)| e.forma != forma).collect();
        self.heap = restantes.into_iter().collect();
    }

    /// Próximo prazo agendado (telemetria/CLI).
    pub fn proximo(&self) -> Option<&Entrada> {
        self.heap.peek().map(|Reverse(e)| e)
    }
}
