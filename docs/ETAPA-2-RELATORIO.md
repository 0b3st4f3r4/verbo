# Etapa 2 — Relatório do Núcleo do Compilador (Rust)

**Escopo:** Etapa 2 do PLAN.md — lexer, parser, AST (FORMAL §2–3), engine de
tick assíncrono com min-heap e relógio virtual injetável, persistência
`equilibrium`→`.vl` canônico com SHA-256, e interpretador de console `.vl` com
FXP simulado (FXP fixado em ponto com 2 casas). ADR-001: linguagem núcleo em
Rust.

**Critério "Done" (AGENTS.md §2.2):**

| Critério | Resultado |
|----------|-----------|
| Matriz de rastreabilidade do parser completa | ✅ 28/28 produções (100%), 9/9 notas semânticas (100%) — [ETAPA-2-MATRIZ-RASTREABILIDADE.md](ETAPA-2-MATRIZ-RASTREABILIDADE.md) |
| Runtime passa em testes de transição | ✅ 36 testes em `nucleo/crates/vbl-runtime/tests/transicao.rs` (reancoragem direta da suíte Python da Etapa 1) |
| Sem vazamentos (ASan/Valgrind) | ✅ 83 testes + E2E do CLI sob AddressSanitizer/LSan, zero relatos |
| Relatório | este documento |

## 1. Arquitetura entregue

Workspace Cargo em `nucleo/` (Rust stable 1.97, sem dependências além de
`sha2` e `criterion` em dev-deps):

```
nucleo/
├── crates/
│   ├── vbl-lang/        lexer, AST, parser, diagnósticos, serialização canônica
│   │   └── tests/       matriz de produções (42) + roundtrip canônico (5)
│   ├── vbl-runtime/     Caderno, FXP (trait + simulador), formas, min-heap,
│   │                    engine de tick, loader, persistência
│   │   ├── tests/       transicao.rs (36 — contrato da Etapa 1 reancorado)
│   │   └── benches/     transicao.rs + escalonador.rs (criterion)
│   └── vbl-cli/         binário `vbl` (tokio): `check` e `run`
└── .cargo-home/         CARGO_HOME local (workspace sandbox; ignorado pelo git)
```

### 1.1 vbl-lang (compilador)

- **Lexer** (§2): unidades léxicas com linha/coluna; strings com escapes e
  limite de 256 bytes; comentários de linha e bloco; operadores de comparação
  de 1 e 2 caracteres; `%`, `°C`, `W`, `°` como lexemas próprios.
- **Parser** (§3): descendente recursivo cobrindo as 28 produções EBNF; AST
  preserva spans e todos os metadados. Duas passadas para cláusulas cruzadas
  (`review_orfa`, `review_duplicada`, `forma_duplicada`, `keep_forma_inexistente`).
- **Diagnósticos**: código canônico + mensagem + linha/coluna (29 códigos de
  parser, cada um com ≥ 1 teste — matriz §3).
- **Serialização canônica** (`canon::form_to_vl`): forma → texto `.vl`
  reparseável, estável, usado pela persistência do runtime; roundtrip
  verificado em testes (inclusive unidades de threshold).

### 1.2 vbl-runtime (motor de transição)

- **Caderno** (`caderno.rs`): cadeia SHA-256 (`hash = SHA256(head ‖ seq ‖ kind
  ‖ msg [‖ extras_json])`, head inicial `0×64`), exportação JSONL, verificação
  de integridade (`verify_chain`), busca por kind/filtro. 18 kinds canônicos
  (`dissolve_rule`…`reclassify_sem_deadline`) nos níveis
  INFO/AVALIACAO/ALERTA/COLAPSO/SUBVERSAO/VAZAMENTO/LEITURA/ATUACAO.
- **FXP** (`fxp.rs`): trait `Fxp` (injetável) + `FxpSimulator` com registro
  mínimo do §6 — sensores `cpu_temp`/`cpu_power`/`attention`, atores
  `CpuPowerCap` [10..250, safety 200], `Ventoinha` [0..255, safety 200],
  `LedIndicador`. Falha de sensor **nunca é 0.0** (§4.7): a condição não é
  avaliada e o alerta é registrado. Fallback por ator (rota de I/O alternativa,
  nunca leitura falsificada). Toda atuação grava **um único** evento ATUACAO.
- **Escalonador** (`scheduler.rs`): min-heap `BinaryHeap<Reverse<Entrada>>`
  por instante; `agendar` O(log N), `drenar_vencidos` O(vencidos),
  `remover_forma` para dissoluções. Versões de prazo descartam entradas
  obsoletas geradas por `keep`/reclassificação (o heap não cresce com o tempo
  — teste `heap_nao_cresce_ao_renovar_manutencao`, 50 ticks).
- **Engine** (`engine.rs`): loop de tick com relógio virtual (1 tick = 1 s,
  configurável); ordem por forma: vazamento P/N → leitura do `source_path` →
  regras de revisão na ordem declarada (antes dos prazos) → manutenção →
  horizon. Dissoluções liberam recursos no mesmo tick (contadores de retenção
  → 0).
- **Persistência** (`persist.rs` + engine): `nonequilibrium→equilibrium`
  grava `<persist-dir>/<forma>.vl` canônico + evento `persistencia` com
  SHA-256/bytes; `cost_bytes` ausente passa a valer o tamanho real gravado;
  sidecar `.json` guarda `creation_time` para o horizon ABSOLUTO sobreviver à
  recarga; inicialização recarrega equilibria cujo horizon ainda não venceu
  (evento `recarga`).

### 1.3 vbl-cli (interpretador de console)

```
vbl check <arquivo.vl>            # parser + validação contra o registro; saída linha:col
vbl run <arquivo.vl> [--ticks N] [--real-ms MS] [--persist-dir DIR]
        [--caderno ARQUIVO.jsonl] [--set SENSOR=VALOR] [--at TICK:SENSOR=VALOR]
        [--permitir-sem-registro]
```

- Runtime tokio (`PLAN §2.2`): o loop de tick é assíncrono; em modo virtual
  os ticks rodam em sequência determinística, em `--real-ms` a cadência é de
  tempo real (1 tick por intervalo).
- `-h/--help` embutido; exit 0 válido, 1 inválido, 2 uso/IO.
- `run` recarrega equilibria do `--persist-dir` antes de executar (retomada de
  estado entre execuções — demonstrado nos exemplos).

## 2. Semântica reancorada da suíte da Etapa 1

Os 36 testes de `transicao.rs` reproduzem o contrato fixado pelos testes
Python (`tests/unit/test_finitude.py`, `test_tick.py`, `test_atores.py`,
`test_falha_sensores.py` e cláusulas de `test_clausulas_erro.py`):

- horizon ABSOLUTO: dissolve com `>=`; reclassificação não renova
  (`creation_time` preservado); equilibria também expiram;
- manutenção: colapso no primeiro vencimento **estrito** (`>`); keep implícito
  a cada tick enquanto houver regra de revisão ativa; keep manual renova;
- ordem no tick: regras avaliadas na ordem declarada **antes** dos prazos;
  `review_short_circuit` sem revogar atuações já despachadas;
- `subvert`: valor poético canônico + dissolução no mesmo tick (≤ 1 tick
  virtual), ações seguintes da mesma regra (ex. `act`) ainda executam;
- `notify_shutdown`: alerta sem dissolução, ações seguintes executam;
- transições: `EQ→EQ` não é transição legal da matriz (FORMAL §4.1) — no-op
  auditado; `EQ/NEQ→NEQ` sem `maintenance_deadline` declarado gera evento
  `reclassify_sem_deadline` e a forma permanece; NEQ→EQ→NEQ preserva o
  deadline **declarado** original;
- vazamento: partilha igual P/N × tick_seconds por forma ativa, somando a
  potência global;
- limites de ator são **inclusivos** (10 e 250 válidos para CpuPowerCap);
  rejeição fora dos limites nunca chega ao ator (`actor_rejected_value`);
  fallback executa após falha do primário com trilha completa.

### 2.1 Refinamentos deliberados face ao protótipo

Documentados como refinamento (comportamento do runtime Rust):

1. **ATUACAO registrado uma única vez** — no protótipo o evento de atuação era
   gravado em camada duplicada; aqui a gravação é responsabilidade exclusiva
   da implementação do FXP (engine não dupla-registra). Efeito: contagem de
   eventos idêntica em semântica, sem ruído de auditoria.
2. **`equilibrium→equilibrium`** — guard explícito: não é transição da matriz
   (§4.1); emite warning e não persiste (o protótipo permitia a regravação).
3. **`unidade_ausente`/`unidade_incompativel`** — viraram diagnósticos de
   carga (`validar` no loader, antes do primeiro tick), não eventos de runtime;
   alinhado ao critério EIF/AGENTS §1.2 (grandeza validada contra o registro).
4. **`exchange_mode`** — anotação de auditoria no registro da forma (PLAN
   §2.2: efeito semântico pleno em definição; `cooperation` é o padrão
   registrado; valor não canônico gera alerta).

## 3. Métricas medidas (AGENTS §1.3)

Benchmarks criterion (`make rust-bench`; máquina de desenvolvimento, modo
`--quick`; p95 = extremo superior do intervalo reportado):

| Benchmark | Medido | Orçamento | Status |
|-----------|--------|-----------|--------|
| `transicao/revisao_dispara_reclassify_1_forma` (regra + reclassificação + persistência `.vl` + SHA-256) | **76,5 µs** | ≤ 100 µs p95 | ✅ |
| `subvert_mesmo_tick` (regra + subvert + act + dissolução) | 37,8 µs | — (referência) | ✅ |
| `fxp_act_local` (por mensagem, 100 iterações/lote) | **2,7 µs** | ≤ 10 µs | ✅ |
| `tick/1` forma | 2,9 µs | — | ✅ |
| `tick/100` | 315 µs | — (linear: ~3 µs/forma, dominado pelo Caderno+SHA-256) | ✅ |
| `tick/1000` | 3,10 ms | — | ✅ |
| `escalonador/agendar` (100→10 000 entradas) | 46→49 ns | O(log N) — crescimento logarítmico confirmado | ✅ |
| `escalonador/drenar_todos` (100→10 000) | 3,7 µs→725 µs | O(vencidos) — linear | ✅ |

O orçamento de memória por forma (§1.3: ≤ 256 B/`event`, ≤ 1 KB/`equilibrium`,
≤ 512 B/`nonequilibrium`) é rastreado em runtime pelos contadores de retenção
(`ORCAMENTO_RETENCAO = (256, 1024, 512)`; teste
`contadores_de_retencao_dentro_dos_orcamentos` verifica os bytes retidos de
cada conjugação e a liberação na dissolução).

## 4. Qualidade de código

- **clippy**: zero warnings em `--workspace --all-targets`
  (`make rust-lint`; CI roda com `-D warnings`).
- **ASan/LSan** (`make rust-asan`, AGENTS §1.3 aceita ASan/Valgrind; miri não
  está instalado na toolchain nightly local): todos os testes (83) e um E2E do
  CLI rodam sob `-Zsanitizer=address` sem relatos de vazamento ou uso de
  memória inválida. Formas dissolvidas liberam recursos no mesmo tick
  (verificação semântica adicional pelos contadores de retenção).
- **Cobertura**: `cargo-llvm-cov` disponível via `make rust-coverage`;
  o denominador canônico de cenários (matriz da FORMAL) está 100% coberto por
  testes dedicados — 42 (parser) + 5 (canon) + 36 (transição).
- **CI**: job `nucleo` em `.github/workflows/ci.yml` (clippy, testes, ASan,
  benchmarks quick) + job Python da Etapa 1.

## 5. Artefatos e como reproduzir

```bash
make rust-check      # clippy (-D warnings) + todos os testes
make rust-asan       # AddressSanitizer
make rust-bench      # criterion
make rust-coverage   # relatório HTML em nucleo/target/coverage

# CLI ponta a ponta:
nucleo/target/debug/vbl check exemplos/exemplo1_pensar_livre.vl
nucleo/target/debug/vbl run exemplos/exemplo2_trading_especulativo.vl \
    --ticks 5 --set cpu_temp=86.5 --persist-dir persistencia --caderno caderno.jsonl
```

Exemplos canônicos em `exemplos/` (PensarLivre, TradingEspeculativo,
TarefaImportante) — validados pelo parser e executáveis pelo `vbl`.

## 6. Limitações conhecidas e próximos passos (Etapa 3)

- O FXP embarcado é **simulado** (CI/`--set`/`--at`); drivers reais
  (sysfs/RAPL/PWM) são a Etapa 3 — a trait `Fxp` é o ponto de extensão.
- `exchange_mode` permanece anotação de auditoria (efeito semântico em
  definição no PLAN §2.2).
- Caderno exporta JSONL; formato binário compacto (Cap'n Proto/FlatBuffers)
  fica para a Etapa 4 (overhead ≤ 1% CPU medido em A/B).
- Estresse de 10 000 formas ativas (GQT) será formalizado na Etapa 5; os
  benchmarks de escalonador já cobrem 100 000 entradas de prazo.
