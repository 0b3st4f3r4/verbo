# ADR-001 — Linguagem do núcleo da VerboLang: **Rust**

| | |
|---|---|
| **Status** | Aceito — decisão registrada na Etapa 1 (docs/PLAN.md §1.3 d) |
| **Data** | 30/08/2026 |
| **Decide** | AD (aprovação) com EC/EIF/AC consultados |
| **Reancora** | Orçamentos de memória e latência do AGENTS.md §1.3 (ver §Orçamentos) |

## Contexto

O PLAN.md §1.3 exige que a Etapa 1 registre a **decisão Rust × C** para o núcleo
da linguagem (lexer, parser, AST e motor de tick assíncrono — entregável da
Etapa 2), com **reancoragem dos orçamentos de memória/latência**. O protótipo
de referência em Python (`prototype/verbolang-complete-blueprint.py`) permanece
como especificação executável; a suíte da Etapa 1 (`tests/`) fixa o contrato
comportamental que a implementação nativa deverá satisfazer (mesmos cenários
BDD e unitários, reancorados por adaptador).

Requisitos que a linguagem precisa sustentar (FORMAL §4, AGENTS §1.3):

1. **"Zero vazamento de heap após dissolução"** (redefinição do zero-heap,
   PLAN §5): toda forma ativa é uma estrutura de vida finita com `horizon`
   explícito; ao dissolver, os recursos são liberados **no mesmo tick** — e o
   critério do AD é verificável por ferramenta (ASan/Valgrind/miri).
2. **Motor de tick assíncrono não bloqueante**: leitura de sensores e envio de
   comandos a atores via FXP sem bloquear o loop (FORMAL §4.2).
3. **Escalonador por fila de prazos** (min-heap por `horizon`/
   `maintenance_deadline`): O(log N) por mutação, varredura O(N + vencidos)
   por tick.
4. **`subvert` como interrupção de prioridade máxima** dentro do tick.
5. **Latência de transição ≤ 100 µs (p95)** na máquina de referência
   (AMD Ryzen 7 7735HS — que é esta máquina de desenvolvimento, 16 núcleos),
   medida com `criterion` em benchmarks dedicados (FORMAL §4.2: a suíte de
   ticks usa relógio virtual; latência de parede sai de benchmarks).
6. **Overhead de integração FXP ≤ 10 µs por mensagem local** (AGENTS §1.3).

## Decisão

Adotar **Rust** como linguagem do núcleo da VerboLang (Etapa 2 em diante).

### Justificativa

| Critério | Rust | C |
|---|---|---|
| Zero vazamento após dissolução | ownership + RAII verificáveis em compilação; `miri`/ASan/LeakSanitizer cobrem o resto | verificação 100% manual + ASan/Valgrind; risco contínuo — é justamente a classe de erro que o projeto proíbe ontologicamente |
| Motor assíncrono não bloqueante | `tokio` (citado no PLAN §2.2) | `epoll`/`poll` manuais (PLAN §2.2) — custo alto de acerto |
| Escalonador min-heap | `BinaryHeap` std + relógio virtual injetável | implementação manual |
| Latência e memória previsíveis | sem GC; abstrações de custo zero (PLAN §5.2) | equivalentes, com mais esforço |
| Tooling de medição do AGENTS §3 | `cargo-llvm-cov`, `clippy`, `criterion` — todos nativos do ecossistema | gcov/lcov, cppcheck, perf — funcionais, mas sem equivalentes de cobertura de invariantes |
| Portabilidade futura para embarcados | `no_std` possível se exigido | superior aqui, mas não é requisito do pipeline |
| Risco de regressão de memória em refatorações | compilador recusa — transformando o critério ontológico "nada de inerte" em invariante de tipo | revisão humana |

**C rejeitado** porque a principal classe de risco do projeto — estruturas
retidas além do `horizon` — é exatamente a que C deixa inteiramente sob
disciplina manual. **Python permanece** apenas como protótipo/espécie
executável (sem orçamentos determinísticos de memória).

**Consequência de fronteira:** drivers FXP que exijam C (sysfs/ioctl/GPU
específicos) poderão ser encapsulados via FFI sem violar a decisão — o núcleo
(parser, AST, runtime, escalonador) é Rust.

## Orçamentos reancorados

Reancoragem dos números do AGENTS.md §1.3 sob Rust, com método de medição
definido (o que era intenção vira instrumento):

| Orçamento | Valor (mantido) | Reancoragem em Rust |
|---|---|---|
| Heap por forma `event` | ≤ 256 B | `size_of::<FormaEvent>()` + capacidade do `value` (≤ 256 B de string, FORMAL §2); alocado em arena por tick |
| Heap por forma `equilibrium` | ≤ 1 KB | `size_of::<FormaEquilibrium>()` + `cost_bytes` do valor; corpo persistido em disco (FORMAL §4.1), não em RAM |
| Heap por forma `nonequilibrium` | ≤ 512 B | `size_of::<FormaNonequilibrium>()` + bookkeeping de manutenção (prazo + último keep) |
| Retenção após dissolução | 0 B | verificação em 3 camadas: testes de contador do runtime (já na suíte da Etapa 1), `miri`/LeakSanitizer na Etapa 2, soak 24 h na Etapa 5 |
| Latência de transição | ≤ 100 µs (p95) | benchmark `criterion` na máquina de referência (Ryzen 7 7735HS), relógio de parede, cenário de revisão com 1 forma e 1 regra |
| Escalonador | O(log N) mutação; O(N + vencidos)/tick | `BinaryHeap<Reverse<Prazo>>`; varredura drena só os vencidos |
| Integração FXP | ≤ 10 µs/mensagem local | canal `mpsc`/`tokio` em processo, medido do despacho ao ack do driver simulado |
| Logging (Caderno) | ≤ 200 µs/evento; ≤ 1 % CPU | escrita assíncrona em buffer + flush periódico (Etapa 4), cadeia SHA-256 incremental |
| Suite de testes | ≤ 15 min completos | Etapa 1 roda em < 1 s (62 unitários + 3 BDD); orçamento vale para a suíte integrada da Etapa 4 |

## Consequências

**Positivas:** invariante ontológico ("nenhuma estrutura sobrevive ao próprio
`horizon`") vira verificação de compilador + teste; benchmarks e cobertura
nativos do pipeline de CI; comunicação FXP local dentro do orçamento de 10 µs
sem serialização binária (schema v1 só entre processos — Etapa 3).

**Negativas / riscos:** curva de aprendizado e tempos de compilação maiores;
mitigação — workspace enxuto, `cargo check` incremental no CI, dependências
zero-runtime no núcleo (somente `tokio` + `criterion` em dev, a definir na
Etapa 2). Orçamentos revisados após a primeira implementação completa
(AGENTS §4), mantida a honestidade termodinâmica.

## Registro na suíte da Etapa 1

Os orçamentos por conjugação já são **testados como contador de retenção do
runtime** (`engine.retained_bytes`, proxy determinístico) em
`tests/unit/test_finitude.py::test_contadores_de_retencao_dentro_dos_orcamentos`
e nos steps do Caso 1/2 do BDD — a implementação Rust substituirá o proxy
pela medição real (`size_of` + arenas), mantendo os mesmos testes como matriz
de rastreabilidade.
