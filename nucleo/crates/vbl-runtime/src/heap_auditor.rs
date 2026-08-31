//! Auditor de heap (Etapa 5 — PLAN §5.1; AGENTS §2.2 "consumo de memória
//! dentro dos limites").
//!
//! Alocador global de CONTAGEM, ativo apenas com a feature `heap-audit`
//! (builds de produção NUNCA a usam): mede heap corrente, pico, total
//! acumulado e número de alocações sem syscalls nem dependências externas —
//! o fechamento físico dos orçamentos de retenção que a ADR-001 deixou como
//! proxy (forma.rs: "a medição física fecha na Etapa 5").
//!
//! Valgrind/Massif não estão disponíveis em todas as máquinas; o auditor é
//! determinístico, roda em CI e responde à pergunta central do PLAN §5.1
//! ("vazamento inerte"): **alguma estrutura permanece em heap além do
//! horizon?** — o teste de churn compara a heap antes/depois de milhares de
//! ciclos de (conjugação → dissolução).
//!
//! Semântica de medição por DELTA: [`zerar`] fixa a linha de base na heap
//! viva corrente; [`atual`] devolve o delta contra ela (pode ser negativo —
//! heap abaixo da base). Os contadores internos nunca são zerados, então
//! `dealloc` de memória pré-existente não subtrai além do real.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

/// Heap viva REAL do processo — nunca zerada; a linha de base fica à parte.
static HEAP_VIVA: AtomicUsize = AtomicUsize::new(0);
static LINHA_DE_BASE: AtomicUsize = AtomicUsize::new(0);
static PICO: AtomicIsize = AtomicIsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static ALOCACOES: AtomicUsize = AtomicUsize::new(0);

/// Alocador real (`System`) com contagem — delega TUDO e apenas anota.
pub struct AuditorAlloc;

unsafe impl GlobalAlloc for AuditorAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            anotar(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            anotar(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        HEAP_VIVA.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(p, layout)
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let novo = System.realloc(p, layout, new_size);
        if !novo.is_null() {
            HEAP_VIVA.fetch_sub(layout.size(), Ordering::Relaxed);
            anotar(new_size);
        }
        novo
    }
}

fn anotar(bytes: usize) {
    let viva = HEAP_VIVA.fetch_add(bytes, Ordering::Relaxed) + bytes;
    let delta = viva as isize - LINHA_DE_BASE.load(Ordering::Relaxed) as isize;
    PICO.fetch_max(delta, Ordering::Relaxed);
    TOTAL.fetch_add(bytes, Ordering::Relaxed);
    ALOCACOES.fetch_add(1, Ordering::Relaxed);
}

/// Fixa a linha de base na heap viva corrente e zera pico/total/alocações
/// (medição por delta entre pontos de quiescência do processo).
pub fn zerar() {
    let viva = HEAP_VIVA.load(Ordering::Relaxed);
    LINHA_DE_BASE.store(viva, Ordering::Relaxed);
    PICO.store(0, Ordering::Relaxed);
    TOTAL.store(0, Ordering::Relaxed);
    ALOCACOES.store(0, Ordering::Relaxed);
}

/// Delta de heap contra a linha de base (bytes; negativo = abaixo da base).
pub fn atual() -> isize {
    HEAP_VIVA.load(Ordering::Relaxed) as isize - LINHA_DE_BASE.load(Ordering::Relaxed) as isize
}

/// Pico de delta de heap desde o último [`zerar`].
pub fn pico() -> isize {
    PICO.load(Ordering::Relaxed)
}

/// Total acumulado alocado desde o último [`zerar`] (throughput de heap).
pub fn total() -> usize {
    TOTAL.load(Ordering::Relaxed)
}

/// Número de alocações desde o último [`zerar`].
pub fn alocacoes() -> usize {
    ALOCACOES.load(Ordering::Relaxed)
}
