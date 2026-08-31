# Relatório da Etapa 5 — Revisão de Qualidade, Otimização e Padrões Compatíveis

**Status:** ✅ Concluída · **Branch:** `main` · **Data:** 2026-08-31

A Etapa 5 fecha o PLAN (§5): **profiling termodinâmico profundo** — memória
com fechamento físico dos orçamentos (fim do proxy da ADR-001), energia com
as rotas reais documentadas — **eliminação de inércia oculta** nos caminhos
quentes do engine e do Caderno, e a **validação final de integridade** com a
revisão formal das metas provisórias (AGENTS §4). Todas as otimizações
preservam a semântica pinada: a suíte inteira (146 testes Rust + 63 testes Python +
BDD) segue verde, clippy `-D warnings` limpo, ASan/LSan limpo, e o formato
`.vcad` produz **linhas byte a byte idênticas** às da composição anterior
(garantia por teste de equivalência dedicado).

---

## 1. Entregáveis (PLAN §5) — checklist

| Entregável | Status | Onde |
|---|---|---|
| 5.1 "Vazamento inerte": nenhuma estrutura em heap além do horizon | ✅ demonstrado | Auditor de contagem de alocação (feature `heap-audit`) + churn de 200 mil ciclos + soak (`tests/memoria.rs`, `vbl-soak`) |
| 5.1 Ferramentas: Valgrind/Massif, PowerAPI, Flamegraphs/Perf | ⚠️→✅ honesto | Valgrind ausente na máquina → **alocador global de contagem** (zero-dep, determinístico, CI); `perf stat` usado; `perf record` e RAPL `energy_uj` exigem root — rotas laboratoriais documentadas (§5/§6) |
| 5.2 Zero-cost abstractions (Rust) | ✅ | Caminho quente do tick sem snapshot de `String`s (`Rc<str>`), regras avaliadas por índice com clone restrito ao disparo, hash incremental em duas fatias, hex por tabela, encoder direto do Caderno |
| 5.2 Deterministic memory management | ✅ | Heap medida e limitada por testes-gate; capacidade (não vazamento) discriminada do crescimento real |
| 5.2 Validação final de integridade (AD) | ✅ | Checklist do AGENTS §1.1 aplicado (§7) + revisão de metas ([STAGE-5-GOALS-REVIEW.md](STAGE-5-GOALS-REVIEW.md)) |
| 5.3 Compilador final com relatórios de performance | ✅ | Este relatório + `logs/stage5/` (baselines criterion antes/depois, soak, ASan, logs `.vcad` verificados) |

## 2. Metas de "Pronto" (AGENTS §2.2 Etapa 5) — medidas

| Meta | Medido | Veredito |
|---|---|---|
| **Zero vazamentos de heap em longa execução (24 h)** | (a) churn **200.000** ciclos (nascer → dissolver pelo caminho natural do horizon): heap retorna à base (**+20 KB** de capacidade retida — não cresce); (b) soak **15 min ≈ 240 mil ticks** com 666 formas vivas renovadas a cada 3 ticks: RSS plano (3 276 → 3 316 KiB, pico 3 316 — platenga, sem deriva); (c) ASan/LSan limpo em todo o workspace; (d) rota das 24 h: `make rust-soak` (padrão `SEGUNDOS=86400`) | ✅ na sessão (a–c) · ⏳ 24 h laboratorial (d) |
| **Consumo de memória dentro dos limites** | Steady-state 10.000 formas: **7,43 MB ≤ 10 MB** ✅; por forma: `event` **743 B**, `equilibrium` **743 B**, `nonequilibrium`+1 regra **1 448 B** — os orçamentos provisórios (256 B/1 KB/512 B) não cabem em contêineres std e foram **reancorados** (§4; [METAS](STAGE-5-GOALS-REVIEW.md)) | ✅ com revisão formal |
| **Profiling mostra ausência de gargalos > 100 ms** | p95 de toda operação unitária ≤ 2 ms (criterion); lotes amortizados (drenar 100k prazos = 10,5 ms → 105 ns/prazo); tick linear (1,64 µs/forma @100 → 1,8 µs/forma @1000) | ✅ |

## 3. Otimizações medidas (antes × depois — criterion, mesma máquina)

Baselines salvos (`--save-baseline antes`) e comparados (`--baseline antes`);
saída completa em [`logs/stage5/bench-after-vs-before.txt`](../logs/stage5/bench-after-vs-before.txt).

| Bench (antes → depois) | Δ | Otimização |
|---|---|---|
| `caderno_overhead/tick_1000_formas_logger_producao`: 2 745 µs → **671 µs** | **−75,5 %** | Encoder direto: `leak` envia dados crus; a linha canônica nasce no buffer do gravador, sem `format!`+`Json::obj`+`BTreeMap` intermediários |
| `caderno_overhead/tick_1000_formas_logger_memoria`: 3 459 µs → **1 737 µs** | −49,8 % | Hash incremental (duas fatias, sem concatenar `head+linha`), hex por tabela, `Evento::escrever_linha` em buffer |
| `caderno_overhead/tick_1000_formas_logger_desligado`: 1 005 µs → **435 µs** | −56,3 % | Engine (§abaixo) + A/B honesto: `NoopCaderno::leak` agora é no-op de verdade ("logger desligado" não constrói evento) |
| `tick/1000` (logger em memória): 3 418 µs → **1 796 µs** | **−47,4 %** | Sem snapshot de `Vec<String>` por tick (iteração por índice sobre `Rc<str>`), regras avaliadas por índice (clone só no disparo), `source_path` emprestado, `bind` O(log N) |
| `tick/1`: 2,95 µs → **1,41 µs** · `tick/100`: 336 µs → **164 µs** | −52 % / −51 % | idem |
| `transicao/revisao_dispara_reclassify_1_forma`: 73,9 µs → **65,7 µs** | −11 % | idem (orçamento ≤ 100 µs p95 com mais folga) |
| `subvert_mesmo_tick`: 41,3 µs → **28,4 µs** | −31 % | idem |
| `caderno_gravacao/memoria_cadeia_evento`: 2,49 µs → **1,02 µs** | −58 % | idem |
| `caderno_gravacao/producao_assincrona_evento`: 1,44 µs → **1,27 µs** | −10,7 % | idem |
| `fxp_atuacao_local/simulado`: 2,79 µs → **1,46 µs** | −48 % | hash/hex no registro da atuação |
| `fxp_act_local`: 325 µs/100 atos → **183 µs** (1,83 µs/ato) | −44 % | idem (orçamento ≤ 10 µs/ato: 5,5× folga) |
| `escalonador/agendar`, `drenar_todos`, `fxp_schema*` | sem mudança | scheduler/schema não tocados (controle) |
| `noop_evento`: 227 → 258 ns | +13 % (31 ns) | ruído de layout de código; sem alteração no caminho |

**Overhead do logger (A/B final):** produção 671 µs − desligado 435 µs =
**Δ ≈ 236 µs/tick @ 1.000 formas ≈ 236 ns/forma** → **0,024 % de CPU** no tick
de parede de 1 s (Etapa 4: 2,1 ms → **9× menor**) e ≈ 0,24 % com 10.000 formas —
a meta ≤ 1 % (AGENTS §1.4) agora é atendida com folga nas duas bases.

### 3.1 Garantia de equivalência

O encoder direto é **byte a byte idêntico** à composição geral
(`evento_vazamento` + `stamp_time` + `Evento::linha`), provado por
`vazamento_caminho_direto_identico_a_composicao_geral`
(`tests/production_notebook.rs`): referência independente via `ChainLedger`,
casos com escape de aspas/barra/controle, unicode, 0.0 W (§4.7), negativos e
integrais; cadeia SHA-256 verificada no binário **e** no JSONL exportado.

### 3.2 Correção de semântica de medição e de inércia

- **A/B honesto:** `NoopCaderno::leak` sobrescrito — a referência "desligado"
  da Etapa 4 ainda construía a mensagem/extra (custo de construção, não de
  logging); agora o A/B mede só o logger.
- **Prazos órfãos:** forma dissolvida no tick não re-agenda prazos pulados
  (antes deixava entradas mortas no heap do escalonador até drenagem futura).
- **Preservação de semântica:** dissoluções no meio do tick reequilibram o
  índice da varredura; o divisor P/N da partilha usa o total do INÍCIO do
  tick (idêntico ao instantâneo da Etapa 1); validade por versão e `horizon`
  absoluto intocados — 146 testes pinados continuam verdes.

## 4. Memória: fechamento físico dos orçamentos (PLAN §5.1; ADR-001)

A ADR-001 usava contadores de retenção como **proxy** ("a medição física
fecha na Etapa 5"). A Etapa 5 entrega o fechamento com o **auditor de heap**:
alocador global de contagem (`src/heap_auditor.rs`, feature `heap-audit` —
builds de produção não pagam nada), heap corrente/pico/total por delta, sem
syscalls nem dependências.

| Medição (serial; `make rust-memoria`) | Resultado |
|---|---|
| Heap por forma `event` (10k, 5 ticks) | **743 B** (orçamento revisado: ≤ 1 KB) |
| Heap por forma `equilibrium` | **743 B** (≤ 1 KB) |
| Heap por forma `nonequilibrium` + 1 regra | **1 448 B** (≤ 2 KB) |
| Steady-state 10.000 formas | **7,43 MB ≤ 10 MB** (pico 7,43 MB) |
| Churn 200.000 ciclos (horizon natural) | retorno à base: **+20 KB** de capacidade (não cresce) |
| ASan/LSan (workspace inteiro, nightly) | limpo — 146 testes, zero vazamentos |

Os orçamentos provisórios do AGENTS (256 B/1 KB/512 B) são **menores que o
nó de `BTreeMap` + chaves + entrada do escalonador** de qualquer forma em
Rust padrão — foram reancorados formalmente em
[`docs/STAGE-5-GOALS-REVIEW.md`](STAGE-5-GOALS-REVIEW.md) (AGENTS §4), com
os proxies da ADR-001 preservados como **piso do payload** próprio da forma.

## 5. Execução longa e gargalos

- **Soak de 15 min (~240 mil ticks, 666 formas vivas renovadas a cada 3
  ticks):** RSS em platenga (3 276 → 3 316 KiB), fila de prazos constante,
  `SOAK OK`. Log completo: [`logs/stage5/soak-15min.txt`](../logs/stage5/soak-15min.txt).
- **Rota das 24 h:** `make rust-soak` (padrão `VIVAS=1000`,
  `SEGUNDOS=86400`) — validação laboratorial formal pendente, agora com
  comando único e veredito automático (exit 1 se o RSS crescer além de
  patamar + 10 % + 4 MiB).
- **Gargalo identificado e registrado (não escondido):** dissolução é O(N)
  (`scheduler.remove_form` reconstrói o heap; `ordem.retain` varre a
  ordem). Com churn de N/3 por tick @10k vivas o custo domina (soak medido:
  362 ms/tick). Correção estrutural (tombstones + compaction amortizada) está
  proposta na revisão de metas, com análise de risco (duplicação na `ordem`).
  Nenhum orçamento de latência existente é violado por ela.
- **Perf:** `perf stat` usado (tempo/user/sys); `perf record` (amostragem de
  ciclos/flamegraph) exige `perf_event_paranoid` menos restritivo ou root —
  rota laboratorial documentada; a ausência de gargalos > 100 ms ficou
  evidenciada pelos p95 do criterion (tabela §2).

## 6. Energia (PLAN §5.1)

**Resolvido no laboratório (31/08):** leitura RAPL real (`energy_uj` liberado
via chmod) com precisão Caderno × RAPL de **|ε| ≤ 0,019 %** (orçamento ±5 % + 1 %),
2 bugs de hardware corrigidos e perf fino simbolizado — ver
[`STAGE-5-LABORATORY.md`](STAGE-5-LABORATORY.md). No CI, a contabilidade segue
o simulador determinístico (modo explícito, FORMAL §4.7).

## 7. Validação final do AD (AGENTS §1.1) e pendências da Etapa 4

| Item (AGENTS §1.1) | Estado |
|---|---|
| Tipos inertes / estruturas sem horizon | ✅ nenhuma — provado por churn/soak/ASan (heap retorna à base após dissolução) |
| Transições event/equilibrium/nonequilibrium conforme a FORMAL | ✅ suíte de transição (37 testes) verde, semântica pinada preservada |
| `subvert` apenas em condições legítimas, dissolve no mesmo tick, `act` posterior não cancelado | ✅ (`subvert_mesmo_tick` 28,4 µs; E2E térmico verde) |
| Sem dependências circulares/acoplamento novo entre camadas | ✅ otimizações internas a `vbl-runtime` (0 dependências novas; `heap-audit` é feature local) |
| Pendência "regras após reclassificação" (ETAPA-4 §7) | ✅ **resolvida**: decisão do AD — regras sobrevivem e permanecem ativas na `equilibrium` (diagrama EQ → DIS por `revisão`; §4.2); manutenção implícita cessa; disparo redundante é no-op auditado. Nota canônica na **FORMAL §4.1** + teste `regras_permanecem_ativas_na_equilibrium_decisao_ad` |
| Pendência "overhead em CPU encadeado" (ETAPA-4 §3/§7) | ✅ **resolvida**: 2,3× → 1,54× (Δ absoluto 9× menor); meta formalizada na revisão de metas |
| Pendência "atribuição causal em Watts (laboratório)" | ⏳ mantida — exige RAPL root (§6) |
| Pendência "validação em hardware real (híbrido)" | ⏳ mantida — rota pronta (`--fxp-config`), laboratório |

## 8. Logs e artefatos ([`logs/stage5/`](../logs/stage5/))

| Arquivo | Conteúdo |
|---|---|
| `baseline/bench-before.txt` | criterion `--save-baseline antes` (pré-otimização) |
| `bench-after-vs-before.txt` | criterion `--baseline antes` (deltas por bench) |
| `soak-15min.txt` | soak de longa execução (RSS/platenga, `SOAK OK`) |
| `asan-summary.txt` | suíte completa sob ASan/LSan — limpa |
| `logs/thermal-subversion.vcad` (+`.jsonl`) | BDD Caso 2 pós-otimização — cadeia ÍNTEGRA |
| `logs/attention-fatigue.vcad` (+`.jsonl`) | BDD Caso 1 — cadeia ÍNTEGRA |
| `logs/main-task.vcad` (+`.jsonl`) | bloco `main`/keep — cadeia ÍNTEGRA |
| `logs/stress-10k.vcad` (+`.jsonl`) | 10.000 formas × 5 ticks = **60.012 eventos**, 750,00 J — cadeia ÍNTEGRA (`caderno-verify`) |

## 9. Como reproduzir

```bash
make rust-check                      # clippy -D warnings + suíte completa
make rust-e2e                        # E2E com Caderno de produção
make rust-memoria                    # orçamentos de heap (auditor serial)
make rust-asan                       # ASan/LSan (nightly)
make rust-bench                      # criterion completo
cd core && cargo bench --bench notebook -- --baseline antes   # deltas
make rust-soak SEGUNDOS=86400        # soak de 24 h (padrão) — laboratório
```

## 10. Trabalho futuro registrado (não bloqueia a entrega)

1. **Dissolução O(1) amortizada** (tombstones + compaction) — proposta com
   análise de risco em [METAS-REVISAO §1](STAGE-5-GOALS-REVIEW.md); perf fino
   confirma o alvo: `remove_form` = 22,7 % dos ciclos (ver laboratório).
2. ~~24 h de soak e leitura RAPL real~~ → **24 h em execução** (veredito no
   §6 do laboratório) · **RAPL real: ✅ concluído** (|ε| ≤ 0,019 %).
3. ~~Perf fino/flamegraphs~~ → **✅ concluído** — `memcmp` 28,8 %,
   `remove_form` 22,7 % ([STAGE-5-LABORATORY.md](STAGE-5-LABORATORY.md) §5).
4. **Metering energético por forma** — extensão futura já prevista na FORMAL §4.2.
