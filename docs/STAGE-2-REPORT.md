# STAGE-2-REPORT.md — Núcleo do Compilador (Rust)

**Escopo:** Etapa 2 do PLAN — lexer, parser, AST (FORMAL §2–3), engine de tick
assíncrono com min-heap e relógio virtual injetável, persistência
`equilibrium`→`.vl` canônico com SHA-256, interpretador de console `.vl` com FXP
simulado (fixed-point, 2 casas). ADR-001: núcleo em Rust.

| Critério "Done" (AGENTS §2.2) | Resultado |
|---|---|
| Matriz de rastreabilidade completa | ✅ 28/28 produções, 9/9 notas semânticas — [STAGE-2-TRACEABILITY-MATRIX.md](STAGE-2-TRACEABILITY-MATRIX.md) |
| Runtime em testes de transição | ✅ 36 testes (`core/crates/vbl-runtime/tests/transition.rs`) — reancoragem direta da suíte Python da Etapa 1 |
| Sem vazamentos (ASan/Valgrind) | ✅ 83 testes + E2E do CLI sob ASan/LSan, zero relatos |

## 1. Arquitetura

Workspace em `core/` (Rust stable, sem deps além de `sha2`; `criterion` em
dev-deps): `vbl-lang` (lexer, AST, parser, diagnósticos, canon) ·
`vbl-runtime` (Ledger, trait `Fxp` + simulador, formas, min-heap, engine,
loader, persistência) · `vbl-cli` (binário `vbl`, tokio).

- **Lexer** (§2): line/col; strings com escapes ≤ 256 B; comentários; operadores
  de 1–2 chars; `%`, `°C`, `W`, `°` como lexemas próprios.
- **Parser** (§3): descendente recursivo, 28 produções; AST preserva spans e
  metadados; duas passadas para cláusulas cruzadas (`review_orfa`,
  `review_duplicada`, `forma_duplicada`, `keep_forma_inexistente`).
  29 códigos de diagnóstico canônicos, cada um com ≥ 1 teste.
- **Canônica** (`canon::form_to_vl`): forma → texto `.vl` reparseável estável
  (usado pela persistência); roundtrip verificado (inclusive unidades).
- **Ledger** (`ledger.rs`): cadeia SHA-256 (`hash = SHA256(head ‖ seq ‖ kind ‖
  msg [‖ extras_json])`, head `0×64`), JSONL, `verify_chain`, busca por
  kind/filtro. 18 kinds canônicos nos níveis INFO/ASSESSMENT/ALERT/COLLAPSE/
  SUBVERSION/LEAK/SENSOR_READ/ACTUATION — normalizados na v1.1 da spec
  ([NOTA DE VERSÃO](NOTEBOOK-FORMAT-v1.md); kinds v1 em PT seguem aceitos pelo
  verificador; eram "kinds em PT até a Fase C" da
  padronização — ver NOTEBOOK-FORMAT-v1).
- **FXP** (`fxp.rs`): trait `Fxp` injetável + `FxpSimulator` com o registro
  mínimo do §6 (`cpu_temp`/`cpu_power`/`attention`; `CpuPowerCap` [10..250,
  safety 200], `Ventoinha` [0..255], `LedIndicador`). Falha de sensor **nunca é
  0.0** (§4.7). Fallback = rota de I/O alternativa, nunca leitura falsificada.
  Toda atuação grava **um único** evento.
- **Escalonador** (`scheduler.rs`): min-heap por instante; agendar O(log N),
  drenar O(vencidos); versões de prazo descartam entradas obsoletas de
  `keep`/reclassificação (heap não cresce com o tempo — teste 50 ticks).
- **Engine**: ordem por forma — vazamento P/N → leitura do `source_path` →
  regras na ordem declarada (antes dos prazos) → manutenção → horizon.
  Dissoluções liberam recursos no mesmo tick (contadores de retenção → 0).
- **Persistência**: NEQ→EQ grava `.vl` canônico + evento com SHA-256/bytes;
  `cost_bytes` ausente = tamanho real gravado; sidecar `.json` guarda
  `creation_time` (horizon ABSOLUTO sobrevive à recarga); inicialização
  recarrega equilibria válidas (evento `recarga`).
- **CLI**: `vbl check` (linha:col) e `vbl run` (`--ticks`, `--real-ms`,
  `--persist-dir`, `--ledger`, `--set`, `--at`, `--allow-unregistered`);
  tokio; exit 0/1/2; retomada de estado entre execuções.

## 2. Contrato reancorado (deltas face ao protótipo)

O contrato da Etapa 1 (horizon absoluto; colapso estrito `>`; regras antes dos
prazos; `subvert` no mesmo tick sem cancelar `act`; partilha P/N; limites
inclusivos; fallback com trilha; `EQ→EQ` ilegal; NEQ sem deadline herda o
último **declarado**) é reproduzido pelos 36 testes — ver STAGE-1-REPORT §3.
Refinamentos deliberados do runtime Rust:

1. **ATUACAO uma única vez** — gravação é responsabilidade exclusiva do FXP
   (engine não dupla-registra).
2. **`equilibrium→equilibrium`** — guard explícito: warning + não persiste.
3. **`unidade_ausente`/`unidade_incompativel`** — diagnósticos de carga
   (loader, antes do 1º tick), não eventos de runtime.
4. **`exchange_mode`** — anotação de auditoria; `cooperation` é o padrão;
   valor não canônico gera alerta.

## 3. Métricas medidas (AGENTS §1.3)

criterion (`make rust-bench`, modo quick; p95 = extremo superior):

| Benchmark | Medido | Orçamento |
|---|---|---|
| regra + reclassify + persistência + SHA-256 | **76,5 µs** | ≤ 100 µs p95 ✅ |
| `subvert` mesmo tick (+ act + dissolução) | 37,8 µs | referência |
| `fxp_act_local` (por mensagem) | **2,7 µs** | ≤ 10 µs ✅ |
| tick/1 · /100 · /1000 formas | 2,9 µs · 315 µs · 3,10 ms | linear ~3 µs/forma (dominado por Ledger+SHA-256) |
| escalonador: agendar 100→10 000 | 46→49 ns | O(log N) confirmado |
| escalonador: drenar 100→10 000 | 3,7 µs→725 µs | O(vencidos) linear |

Orçamento de memória por forma (≤ 256 B `event`, ≤ 1 KB `equilibrium`,
≤ 512 B `nonequilibrium`) rastreado pelos contadores de retenção
(`RETENTION_BUDGET = (256, 1024, 512)`), com teste de liberação na dissolução.

## 4. Qualidade

clippy `-D warnings` zero · ASan/LSan em 83 testes + E2E (miri ausente na
nightly local — AGENTS aceita ASan/Valgrind) · cobertura via
`make rust-coverage` (denominador canônico 100%: 42 parser + 5 canon +
36 transição) · CI: job `core` + job Python.

## 5. Reprodução

`make rust-check` (clippy+testes) · `rust-asan` · `rust-bench` ·
`rust-coverage`. E2E: `vbl check examples/example1_free_thinking.vl`;
`vbl run examples/example2_speculative_trading.vl --ticks 5 --set cpu_temp=86.5`.

## 6. Limitações → Etapa 3

FXP simulado (drivers reais sysfs/RAPL/PWM → Etapa 3; a trait `Fxp` é o ponto
de extensão) · `exchange_mode` como anotação · formato binário compacto do
Caderno → Etapa 4 · estresse de 10 000 formas formalizado na Etapa 5 (o bench
do escalonador já cobre 100 000 entradas).
