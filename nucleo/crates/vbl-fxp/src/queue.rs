//! Fila de comandos a atores (PLAN §3.4): prioridade, ordem estável e
//! capacidade limitada. A expiração (`queue_timeout`) é aplicada pelo bus no
//! relógio virtual — ver [`crate::bus`].

use std::collections::BinaryHeap;
use std::cmp::Reverse;
use vbl_runtime::fxp::Value;

/// Reexportado do runtime — os constantes canônicos vivem junto do trait
/// `Fxp` (usado pelo engine para a atuação pós-`subvert`).
pub use vbl_runtime::fxp::{PRIORIDADE_NORMAL, PRIORIDADE_SUBVERT};

/// Comando pendente de entrega.
#[derive(Debug, Clone, PartialEq)]
pub struct Comando {
    /// `seq` FXP da mensagem original (correlação ack).
    pub seq: u32,
    pub ator: String,
    pub valor: Value,
    pub prioridade: u8,
    /// Ticks virtuais já aguardando na fila (o bus expira conforme config).
    pub ticks_esperando: u64,
    /// Ator primário original (para auditoria de fallbacks em cascata).
    pub primario: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroFila {
    /// Guarda anti-inchaço: mais pendências do que o orçamento.
    Cheia { capacidade: usize },
}

impl std::fmt::Display for ErroFila {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErroFila::Cheia { capacidade } => {
                write!(f, "fila de comandos cheia (capacidade {capacidade})")
            }
        }
    }
}

impl std::error::Error for ErroFila {}

/// Ordem do heap: prioridade numérica menor = entregue antes; dentro da
/// mesma prioridade, FIFO pelo seq.
///
/// A ordenação total usa **apenas** `(prioridade, seq)` — o `Value` do
/// comando (com f64) nunca entra na comparação, o que torna o `Eq` manual
/// legítimo para a chave (seq é único por mensagem do FXP).
#[derive(Debug, Clone, PartialEq)]
struct Entrada {
    prioridade: u8,
    seq: u32,
    cmd: Comando,
}

impl Eq for Entrada {}

impl Ord for Entrada {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        Reverse((self.prioridade, self.seq)).cmp(&Reverse((other.prioridade, other.seq)))
    }
}

impl PartialOrd for Entrada {
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
pub struct FilaComandos {
    heap: BinaryHeap<Entrada>,
    capacidade: usize,
}

impl Default for FilaComandos {
    fn default() -> Self {
        Self::com_capacidade(256)
    }
}

impl FilaComandos {
    pub fn com_capacidade(capacidade: usize) -> Self {
        Self { heap: BinaryHeap::new(), capacidade }
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    pub fn capacidade(&self) -> usize {
        self.capacidade
    }

    /// Enfileira comando (prioridade define a ordem de re-entrega).
    pub fn empilhar(&mut self, mut cmd: Comando) -> Result<(), ErroFila> {
        if self.heap.len() >= self.capacidade {
            return Err(ErroFila::Cheia { capacidade: self.capacidade });
        }
        cmd.prioridade = cmd.prioridade.min(PRIORIDADE_NORMAL);
        self.heap.push(Entrada { prioridade: cmd.prioridade, seq: cmd.seq, cmd });
        Ok(())
    }

    /// Entrega o próximo comando em ordem de prioridade (menor primeiro).
    pub fn proximo(&mut self) -> Option<Comando> {
        self.heap.pop().map(|e| e.cmd)
    }

    /// Re-enfileira comando que falhou na re-entrega, somando 1 tick de espera.
    pub fn devolver(&mut self, mut cmd: Comando) -> Result<(), ErroFila> {
        cmd.ticks_esperando += 1;
        self.empilhar(cmd)
    }

    /// Esvazia (desligamento limpo; comandos pendentes são descartados com
    /// evento de auditoria pelo bus).
    pub fn limpar(&mut self) {
        self.heap.clear();
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn cmd(seq: u32, prioridade: u8) -> Comando {
        Comando {
            seq,
            ator: "Ventoinha".into(),
            valor: Value::Num(100.0),
            prioridade,
            ticks_esperando: 0,
            primario: None,
        }
    }

    #[test]
    fn prioridade_maxima_entrega_primeiro_e_fifo_dentro_da_mesma() {
        let mut fila = FilaComandos::default();
        for seq in [3u32, 1, 2] {
            fila.empilhar(cmd(seq, PRIORIDADE_NORMAL)).unwrap();
        }
        fila.empilhar(cmd(9, PRIORIDADE_SUBVERT)).unwrap();

        let ordem: Vec<u32> = (0..4).filter_map(|_| fila.proximo()).map(|c| c.seq).collect();
        assert_eq!(ordem, vec![9, 1, 2, 3], "subvert (0) antes; FIFO dentro de normal (10)");
    }

    #[test]
    fn capacidade_e_guarda_anti_inchaco() {
        let mut fila = FilaComandos::com_capacidade(2);
        fila.empilhar(cmd(1, PRIORIDADE_NORMAL)).unwrap();
        fila.empilhar(cmd(2, PRIORIDADE_NORMAL)).unwrap();
        assert!(matches!(
            fila.empilhar(cmd(3, PRIORIDADE_NORMAL)),
            Err(ErroFila::Cheia { capacidade: 2 })
        ));
        assert_eq!(fila.len(), 2);
    }

    #[test]
    fn devolver_soma_tick_de_espera_e_prioridade_e_saturada() {
        let mut fila = FilaComandos::default();
        fila.empilhar(cmd(1, 200)).unwrap(); // acima do normal: satura em 10
        let c = fila.proximo().unwrap();
        assert_eq!(c.prioridade, PRIORIDADE_NORMAL);
        fila.devolver(c).unwrap();
        let c = fila.proximo().unwrap();
        assert_eq!(c.ticks_esperando, 1, "re-enfileirado conta o tempo na fila");
    }
}
