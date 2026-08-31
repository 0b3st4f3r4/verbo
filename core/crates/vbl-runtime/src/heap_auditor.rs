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
//! Semântica de medição por DELTA: [`zero`] fixa a linha de base na heap
//! viva corrente; [`current`] devolve o delta contra ela (pode ser negativo —
//! heap abaixo da base). Os contadores internos nunca são zerados, então
//! `dealloc` de memória pré-existente não subtrai além do real.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

/// Heap viva REAL do processo — nunca zerada; a linha de base fica à parte.
static HEAP_ALIVE: AtomicUsize = AtomicUsize::new(0);
static BASELINE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

/// Alocador real (`System`) com contagem — delega TUDO e apenas anota.
pub struct AuditorAlloc;

unsafe impl GlobalAlloc for AuditorAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            note(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            note(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        HEAP_ALIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(p, layout)
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = System.realloc(p, layout, new_size);
        if !new.is_null() {
            HEAP_ALIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            note(new_size);
        }
        new
    }
}

fn note(bytes: usize) {
    let alive = HEAP_ALIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    let delta = alive as isize - BASELINE.load(Ordering::Relaxed) as isize;
    PEAK.fetch_max(delta, Ordering::Relaxed);
    TOTAL.fetch_add(bytes, Ordering::Relaxed);
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
}

/// Fixa a linha de base na heap viva corrente e zera pico/total/alocações
/// (medição por delta entre pontos de quiescência do processo).
pub fn zero() {
    let alive = HEAP_ALIVE.load(Ordering::Relaxed);
    BASELINE.store(alive, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    TOTAL.store(0, Ordering::Relaxed);
    ALLOCATIONS.store(0, Ordering::Relaxed);
}

/// Delta de heap contra a linha de base (bytes; negativo = abaixo da base).
pub fn current() -> isize {
    HEAP_ALIVE.load(Ordering::Relaxed) as isize - BASELINE.load(Ordering::Relaxed) as isize
}

/// Pico de delta de heap desde o último [`zero`].
pub fn peak() -> isize {
    PEAK.load(Ordering::Relaxed)
}

/// Total acumulado alocado desde o último [`zero`] (throughput de heap).
pub fn total() -> usize {
    TOTAL.load(Ordering::Relaxed)
}

/// Número de alocações desde o último [`zero`].
pub fn allocations() -> usize {
    ALLOCATIONS.load(Ordering::Relaxed)
}
