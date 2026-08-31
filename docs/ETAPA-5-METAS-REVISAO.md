# Revisão de Metas Provisórias — Etapa 5 (AGENTS §4)

**Status:** proposta consolidada na revisão da Etapa 5; vigente até a próxima
revisão formal. **Máquina de referência:** AMD Ryzen 7 7735HS, 16 threads,
Linux 7.0.0-29, rustc 1.97.1 (release + `lto`? não — perfil padrão release),
 criterion 0.5. **Método:** benches criterion (`make rust-bench`, baselines
`antes`/`depois` em `logs/etapa5/`) e auditor de heap por contagem de
alocação (`make rust-memoria` — feature `heap-audit`, medição serial).

O AGENTS §4 prevê esta revisão: as metas numéricas eram **provisórias** e
devem ser reancoradas nos resultados reais da primeira implementação
completa, mantendo a honestidade termodinâmica. Nenhum número abaixo foi
estimado: todos saem de ferramentas (criterion, auditor de heap, RSS de
`/proc`, verificador do Caderno).

---

## 1. Núcleo (EC — AGENTS §1.3)

| Meta provisória | Medido (Etapa 5) | Veredito | Revisão proposta |
|---|---|---|---|
| Transição ≤ 100 µs p95 | **65,7 µs** (revisão que dispara + reclassify + persistência; era 73,9 µs na Etapa 4) | ✅ mantém | Manter ≤ 100 µs p95 |
| Memória por forma: ≤ 256 B `event`, ≤ 1 KB `equilibrium`, ≤ 512 B `nonequilibrium` | **743 B / 743 B / 1 448 B** (heap real: contêineres std + chaves + entradas do escalonador; `tests/memoria.rs`) | ⚠️ orçamentos irreais para contêineres std — o nó de `BTreeMap` + chaves já excedem 256 B | **Reancorar:** heap total por forma `event` ≤ 1 KB, `equilibrium` ≤ 1 KB, `nonequilibrium` + 1 regra ≤ 2 KB. O contador-proxy da ADR-001 (96/128/160 B) permanece como **piso** do payload próprio |
| Steady-state ≤ 10 MB @ 10.000 formas (PLAN §5) | **7,43 MB** @ 10k `event` (pico 7,43 MB) | ✅ mantém com folga | Manter; adicionar teste como gate (`make rust-memoria`) |
| Escalonador O(log N)/mutação, O(N+vencidos)/tick | agendar 45 ns (estável 100→100k); drenar 105 ns/prazo amortizado | ✅ mantém | Manter |
| **Dissolução O(N)** — **gargalo identificado**, fora dos orçamentos originais | `remover_forma` reconstrói o heap e `ordem.retain` varre a ordem **por dissolução**; com churn de N/3 por tick o custo domina | ⚠️ novo | Registrar otimização estrutural (tombstones + compaction amortizada) para etapa futura; risco: duplicação de nomes na `ordem` exige dedução por época |

## 2. Caderno (AC — AGENTS §1.4)

| Meta provisória | Medido (Etapa 4) | Medido (Etapa 5) | Revisão proposta |
|---|---|---|---|
| Overhead de logging ≤ 1% CPU | 0,2% CPU (parede 1 s/1k formas) mas **2,3×** na base encadeada; ~2% @ 10k | **Δ = 236 µs/tick @ 1k formas** (produção 671 µs × desligado 435 µs) → **0,024% CPU** na parede; ~0,24% @ 10k; 1,54× na encadeada | Manter ≤ 1% (parede) **e** acrescentar teto **≤ 2×** na base encadeada (hoje 1,54×) |
| Latência de gravação ≤ 200 µs/evento | 1,5 µs | **1,27 µs** (−10,7%; encoder direto sem `Json`) | Manter |
| Memória ≤ 5 MB @ 10k formas | ≲ 1 MB | ≲ 1 MB (inalterado — `--caderno` não remeclado; encoder direto só reduz) | Manter |
| Cobertura de eventos 100% / robustez 99,99% | 60.000/60.000 verificados | inalterado (suíte Etapa 4 continua verde) | Manter |

O custo de construção do evento era o gargalo dominante (ETAPA-4-RELATORIO
§3). O caminho quente `leak` agora envia dados crus e a linha canônica é
composta direto no buffer do gravador, com hash incremental sobre duas fatias
e hex por tabela — **linhas byte a byte idênticas**, garantido por teste de
equivalência (`vazamento_caminho_direto_identico_a_composicao_geral`).

## 3. FXP (EIF — AGENTS §1.3)

| Meta provisória | Medido | Veredito |
|---|---|---|
| Ato local ≤ 10 µs | **1,83 µs**/ato (era 3,25 µs) | ✅ |
| Leitura local ≤ 1 ms | 69 ns (simulado) / 6,4 µs (fixture real) | ✅ |
| Leitura remota ≤ 10 ms | 11,8 µs (roundtrip unix) | ✅ |

## 4. Qualidade e execução longa (GQT — AGENTS §2.2)

| Meta provisória | Medido na Etapa 5 | Revisão proposta |
|---|---|---|
| Zero vazamentos de heap em longa execução (24 h) | (a) churn **200 mil** ciclos vida→dissolução: heap retorna à base (+20 KB de capacidade, não cresce); (b) soak **15 min** ≈ 240 mil ticks, 666 formas vivas renovadas a cada 3 ticks: RSS estável (3 276 KiB plano, pico 3 312); (c) ASan/LSan limpo; (d) rota de 24 h: `make rust-soak` (padrão `SEGUNDOS=86400`) | Aceitar (a)–(c) como gate de CI/sessão **e** exigir a corrida de 24 h (d) como validação laboratorial formal |
| Profiling mostra ausência de gargalos > 100 ms | p95 de toda operação unitária ≤ 2 ms (criterion); lotes amortizados: drenar 100k prazos = 10,5 ms (105 ns/prazo); varredura de tick linear (1,64 µs/forma @100 → 1,8 µs/forma @1000) | Manter; `perf record` (amostragem de ciclos) exige `perf_event_paranoid` menos restritivo/root — rota laboratorial documentada no relatório |
| Vazamento inerte (PLAN §5.1) | Nenhuma estrutura sobrevive ao horizon: auditor de contagem + churn + soak provam retorno da heap à base | Gate permanente via `make rust-memoria` |

## 5. Energia (PLAN §5.1 — PowerAPI/RAPL)

O barramento RAPL existe nesta máquina (`/sys/class/powercap/intel-rapl*`),
porém `energy_uj` é legível apenas por root (`0400`). A validação com hardware
real — leitura de potência real durante carga e a atribuição causal em Watts
(≤ 0,1 W, AGENTS §1.4) — **permanece pendente de laboratório**, como já
registrado na Etapa 4. Rota de laboratório (documentação honesta, sem
simulação de dado):

```bash
sudo cat /sys/class/powercap/intel-rapl:0/energy_uj   # antes
<executar a carga>
sudo cat /sys/class/powercap/intel-rapl:0/energy_uj   # depois
# Δenergia = Δ(energy_uj)·1e-6 J (com wraparound em max_energy_range_uj)
```

No CI/sessão, a contabilidade energética do runtime continua apoiada no
simulador determinístico do FXP (modo simulado explícito, FORMAL §4.7).

## 6. Decisões ontológicas registradas nesta revisão

1. **Regras de revisão permanecem ativas na `equilibrium`** (pendência da
   Etapa 4): fundamentada no diagrama de estados da FORMAL (EQ → DIS por
   `revisão`) e em §4.2; no-op auditado quando a ação não altera o estado.
   Nota canônica adicionada à FORMAL §4.1 e teste de fixação
   (`regras_permanecem_ativas_na_equilibrium_decisao_ad`).
2. **Orçamentos de memória por forma** passam a ser medidos por heap real
   (auditor de contagem), não por proxy — os valores da ADR-001 continuam
   válidos como piso do payload.
3. **Dissolução O(N)** declarada como gargalo conhecido, com proposta de
   correção estrutural — não escondido dos orçamentos.
