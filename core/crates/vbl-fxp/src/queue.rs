//! Fila de comandos a atores (PLAN §3.4): prioridade, ordem estável e
//! capacidade limitada. A expiração (`queue_timeout`) é aplicada pelo bus no
//! relógio virtual — ver [`crate::bus`].

use std::collections::BinaryHeap;
use std::cmp::Reverse;
use vbl_runtime::fxp::Value;

/// Reexportado do runtime — os constantes canônicos vivem junto do trait
/// `Fxp` (usado pelo engine para a atuação pós-`subvert`).
pub use vbl_runtime::fxp::{PRIORITY_NORMAL, PRIORITY_SUBVERT};

/// Comando pendente de entrega.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// `seq` FXP da mensagem original (correlação ack).
    pub seq: u32,
    pub actor: String,
    pub value: Value,
    pub priority: u8,
    /// Ticks virtuais já aguardando na fila (o bus expira conforme config).
    pub ticks_waiting: u64,
    /// Ator primário original (para auditoria de fallbacks em cascata).
    pub primary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    /// Guarda anti-inchaço: mais pendências do que o orçamento.
    Full { capacity: usize },
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueError::Full { capacity } => {
                write!(f, "fila de comandos cheia (capacidade {capacity})")
            }
        }
    }
}

impl std::error::Error for QueueError {}

/// Ordem do heap: prioridade numérica menor = entregue antes; dentro da
/// mesma prioridade, FIFO pelo seq.
///
/// A ordenação total usa **apenas** `(priority, seq)` — o `Value` do
/// comando (com f64) nunca entra na comparação, o que torna o `Eq` manual
/// legítimo para a chave (seq é único por mensagem do FXP).
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    priority: u8,
    seq: u32,
    cmd: Command,
}

impl Eq for Entry {}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        Reverse((self.priority, self.seq)).cmp(&Reverse((other.priority, other.seq)))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Fila prioritária de comandos a re-entregar.
///
/// Política (PLAN §3.4/§3 — mitigação): um comando entra na fila quando a
/// entrega inicial esgota retry+fallback do registro; o bus tenta de novo a
/// cada tick, na ordem de prioridade, até expirar o `queue_timeout`.
#[derive(Debug, Clone)]
pub struct CommandQueue {
    heap: BinaryHeap<Entry>,
    capacity: usize,
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::with_capacity(256)
    }
}

impl CommandQueue {
    pub fn with_capacity(capacity: usize) -> Self {
        Self { heap: BinaryHeap::new(), capacity }
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Enfileira comando (prioridade define a ordem de re-entrega).
    pub fn enqueue(&mut self, mut cmd: Command) -> Result<(), QueueError> {
        if self.heap.len() >= self.capacity {
            return Err(QueueError::Full { capacity: self.capacity });
        }
        cmd.priority = cmd.priority.min(PRIORITY_NORMAL);
        self.heap.push(Entry { priority: cmd.priority, seq: cmd.seq, cmd });
        Ok(())
    }

    /// Entrega o próximo comando em ordem de prioridade (menor primeiro).
    pub fn dequeue(&mut self) -> Option<Command> {
        self.heap.pop().map(|e| e.cmd)
    }

    /// Re-enfileira comando que falhou na re-entrega, somando 1 tick de espera.
    pub fn requeue(&mut self, mut cmd: Command) -> Result<(), QueueError> {
        cmd.ticks_waiting += 1;
        self.enqueue(cmd)
    }

    /// Esvazia (desligamento limpo; comandos pendentes são descartados com
    /// evento de auditoria pelo bus).
    pub fn clear(&mut self) {
        self.heap.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(seq: u32, priority: u8) -> Command {
        Command {
            seq,
            actor: "Ventoinha".into(),
            value: Value::Num(100.0),
            priority,
            ticks_waiting: 0,
            primary: None,
        }
    }

    #[test]
    fn max_priority_delivers_first_fifo_within_same() {
        let mut queue = CommandQueue::default();
        for seq in [3u32, 1, 2] {
            queue.enqueue(cmd(seq, PRIORITY_NORMAL)).unwrap();
        }
        queue.enqueue(cmd(9, PRIORITY_SUBVERT)).unwrap();

        let order: Vec<u32> = (0..4).filter_map(|_| queue.dequeue()).map(|c| c.seq).collect();
        assert_eq!(order, vec![9, 1, 2, 3], "subvert (0) antes; FIFO dentro de normal (10)");
    }

    #[test]
    fn capacity_and_anti_bloat_guard() {
        let mut queue = CommandQueue::with_capacity(2);
        queue.enqueue(cmd(1, PRIORITY_NORMAL)).unwrap();
        queue.enqueue(cmd(2, PRIORITY_NORMAL)).unwrap();
        assert!(matches!(
            queue.enqueue(cmd(3, PRIORITY_NORMAL)),
            Err(QueueError::Full { capacity: 2 })
        ));
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn requeue_sum_wait_ticks_plus_priority_saturates() {
        let mut queue = CommandQueue::default();
        queue.enqueue(cmd(1, 200)).unwrap(); // acima do normal: satura em 10
        let c = queue.dequeue().unwrap();
        assert_eq!(c.priority, PRIORITY_NORMAL);
        queue.requeue(c).unwrap();
        let c = queue.dequeue().unwrap();
        assert_eq!(c.ticks_waiting, 1, "re-enfileirado conta o tempo na fila");
    }
}
