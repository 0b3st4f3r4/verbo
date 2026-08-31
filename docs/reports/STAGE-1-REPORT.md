# STAGE-1-REPORT.md — Pesquisa & Arquitetura de Escopo via Código (TDD/BDD/E2E)

| | |
|---|---|
| **Status** | Concluída — critérios de "Done" do AGENTS.md §2.2 atendidos |
| **Data** | 30/08/2026 |
| **CI (§1.3 a)** | GitHub Actions (`.github/workflows/ci.yml`) + alvo local `make test` — a mesma suíte nos dois lugares |
| **Linguagem (§1.3 d)** | **Rust** — [`docs/adrs/ADR-001-linguagem-nucleo.md`](../adrs/ADR-001-linguagem-nucleo.md), orçamentos reancorados |

## 1. Escopo entregue (PLAN §1.1–§1.3)

| Item | Entrega |
|---|---|
| §1.1 BDD | 3 cenários canônicos em `tests/features/*.feature` (`# language: en`), com mocks via **behave**: 17 steps passando |
| §1.2 TDD | 63 testes pytest em `tests/unit/`: finitude, falha controlada de sensores, comandos a atores, ordem do tick, integridade do Caderno, validador de superfície |
| §1.3 b — fronteira mock × simulador | `tests/fxp_sim/mocks.py` (**MockFXP**, dicionários em processo) × `tests/fxp_sim/simulator.py` (**FXPSimulator** — séries roteirizadas, injeção de falhas, fallback no registro, efeitos físicos determinísticos). Schema binário v1 → Etapa 3 |
| §1.3 c — banco de 20 prompts | `docs/cheatsheet/CHEATSHEET-PROMPTS.yaml` (S01–S10 sintaxe, M01–M10 semântica) + `scripts/validate_cheatsheet.py` (3 execuções/prompt, verificador `tests/vlcheck.py`, relatório em `docs/cheatsheet/CHEATSHEET-VALIDATION.md`, limiar ≥ 90 %) |

**SUT:** o protótipo Python (`prototype/verbolang-complete-blueprint.py`) é a
especificação executável; runtime recebe FXP e relógio virtual **injetados**
(FORMAL §4.2 — determinismo total, sem `random`). **Camadas:** unitários cobrem
unidades + contrato; BDD é o E2E **sobre mocks** (Caso 1 atravessa
leitura → regra → reclassificação → persistência → auditoria). E2E com
hardware real → Etapa 4/5.

## 2. Critério "Done" — ≥ 1 teste por cláusula de erro

6/6 cláusulas obrigatórias (AGENTS §2.2) cobertas, mais 9 adicionais — mapa
completo arquivo-a-teste no histórico do git (este arquivo, pré-compressão):

| Cláusula (FORMAL) | Onde testado |
|---|---|
| Sensor ausente — não avalia, não dispara falso (§4.7) | `test_sensor_failures.py`, `test_error_clauses.py` |
| Ator inexistente rejeitado com registro (§4.3) | `test_actors.py`, `test_error_clauses.py` |
| Valor fora de limite rejeitado **sem envio**; rejeição não dissolve (§4.3) | `test_actors.py` |
| Forma sem `value`/`horizon` rejeitada (texto e IR) | `test_error_clauses.py` |
| Review órfã/duplicada rejeitada (texto e IR) | `test_error_clauses.py` |
| `keep` de forma inexistente/dissolvida registrado | `test_error_clauses.py` |

Adicionais pinadas: `maintenance_deadline` obrigatório em `nonequilibrium`;
`cost_bytes`/`exchange_mode` por conjugação; `source_path` só simbólico;
unidade do threshold incompatível com a grandeza; duração inválida; string
> 256 B; ação/operador/atributo desconhecidos; vírgula final;
reclassify para `nonequilibrium` sem deadline (erro de runtime).

## 3. Rastreabilidade FORMAL → teste (resumo)

- **§4.1**: `event` expira no `horizon` com fim tipificado; `horizon` **absoluto**
  (reclassificação não renova); `equilibrium` também expira; manutenção implícita
  com regra ativa; colapso no primeiro vencimento sem manutenção; `keep()` renova;
  fim único por tick; persistência `.vl` canônico + SHA-256.
- **§4.2 tick**: regras na ordem declarada **antes** dos prazos;
  `review_short_circuit` sem revogar atuações; partilha igual P/N;
  relógio virtual injetável; cadeia SHA-256 detecta adulteração e o JSONL a
  reproduz.
- **§4.3/§4.5/§4.6**: mensagem FXP serializada ao ator correto; limites
  inclusivos; `subvert` = valor poético + dissolução no mesmo tick + `act`
  da mesma regra NÃO cancelado; fallback por política do registro + trilha no
  Caderno; `notify_shutdown` não dissolve nem interrompe.
- **§4.7**: `0.0` é leitura válida que dispara regras; sensor ausente/inacessível
  ⇒ condição não avaliada + alerta; forma sem `source_path` não gera leitura.
- Os 6 exemplos canônicos da FORMAL validam no `vlcheck`; programa persistido é
  reparseável.

## 4. Interpretações registradas (lacunas da spec resolvidas em teste)

Ambiguidades da FORMAL cuja leitura a suíte fixa (sujeitas a veto do AD):

1. **"0 bytes retidos em heap" (Caso 1):** a forma reclassificada permanece
   `equilibrium` — que vive em suporte não volátil. Interpretação: o estado
   **laborativo** é integralmente liberado (`labor_registry` → 0); o que
   permanece é a estrutura persistida, dentro do orçamento de retenção.
2. **`reclassify_as_nonequilibrium` sem deadline declarado:** herda o último
   `maintenance_deadline` **declarado** (NEQ → EQ → NEQ é legal com o prazo
   original); nunca declarou ⇒ erro de runtime registrado e a forma permanece.
3. **"Colapsa no primeiro vencimento":** vencimento = tempo desde a última
   manutenção **estritamente maior** que o deadline (no limite exato ainda
   sustenta); já "expirar por `horizon`" é `>=` (no limite expira).
4. **Regras sobrevivem à reclassificação** (a `review` é copiada; pode
   re-disparar na nova conjugação) — semântica plena de conservação → Etapa 2.
5. **`ReserveFan`** é extensão opcional do registro e não entra no
   denominador de cobertura de dispositivos do AGENTS (só os obrigatórios do
   FORMAL §6 contam).

## 5. Divergências do protótipo corrigidas (auditado pela suíte)

1. FXP e passo do relógio **injetáveis** no engine.
2. Fins de forma **tipificados** no Caderno (`dissolve_rule`, `dissolve_horizon`,
   `collapse_maintenance`, `dissolve_subvert`).
3. `review_short_circuit` e `review_after_dissolution` registrados.
4. `actor_rejected_value` na rejeição sem envio (antes: print + False).
5. `keep_forma_inexistente` e `keep_ignorado` registrados (antes: silêncio).
6. Persistência `.vl` canônica + evento com caminho, SHA-256 e `cost_bytes`
   real (antes: `disk_bytes_used += 1024` fixo).
7. **`horizon` absoluto** na reclassificação — o protótipo o RENOVAVA (Lei 1).
8. Reclassify para NEQ sem deadline recusado com erro (antes criava com
   deadline fixo 3 s).
9. Manutenção implícita enquanto houver regra ativa (antes só `keep` do `main`).
10. Partilha igual P/N do vazamento (antes: potência cheia só em `PowerWatts`).
11. `subvert` sem efeito físico no runtime — física é do mundo (FORMAL §4.5).
12. `Caderno.reset()`/`event()` genéricos + campos extras em alertas (kinds e
    motivos consultáveis).

## 6. Pendências assumidas (destino)

Recarregar `equilibrium` persistidas (Etapa 2) · schema binário FXP +
transporte local×remoto (Etapa 3) · aliases e drivers reais com latência p95 e
orçamento de erro (Etapa 3) · Caderno de produção JSONL assíncrono ≤ 1 %
(Etapa 4) · heap real via miri/ASan + soak 24 h (Etapas 2 e 5) · semântica
plena de `exchange_mode` e conservação de regras (Etapa 2) · cheat sheet ≥ 90 %
(então 73,3 % — ver §7 e `CHEATSHEET-VALIDATION.md`; atingido na campanha v4,
93,3 %, durante a Etapa 5).

## 7. Auditoria do cheat sheet (PLAN §7) — campanhas v1–v3

Banco fixo × qwen3-4b (vLLM, cheat sheet injetado), 3 execuções/prompt:
**45 % → 71,7 % → 73,3 %** (v3: semântica 28/30, sintaxe 16/30). O valor
desta campanha está na **triagem por causa**, não na nota:

| Causa | Evidência | Ação |
|---|---|---|
| Lacuna do cheat sheet | M02, M06, M08, M09 não ensinados | 4 adições auditadas contra a FORMAL |
| Rubrica de formatação exata | S02, S05 | âncoras estruturais + itens negativos sobre o bloco de código |
| Prompt ambíguo | S05/S08 pediam fragmento (validador certo, prompt errado) | prompts passam a exigir programa completo |
| Erro real do modelo | `cost_bytes` em NEQ, vírgula final, formas/ações inventadas | falha legítima — é o ruído que o limiar de 90 % mede |

**Veredito v3:** limiar não atingido; gargalo = fiabilidade do modelo local
diante da gramática. Consequência operacional (AGENTS §5): até atingir o
limiar, toda saída do modelo local usada como artefato passa pela validação do
GQT. (Resolvido na v4 — 93,3 % ACEITO, ver `CHEATSHEET-VALIDATION.md`.)

## 8. Como executar

`make setup` (venv) · `make test` (pytest + behave) · `make check` (estático) ·
`make validate-cheatsheet` (manual, exige o LLM local via `make serve`). CI:
GitHub Actions roda `make check` + pytest + behave a cada push/PR (Python
3.13); a validação do cheat sheet é manual — o modelo não sobe no CI.

## 9. Métricas da suíte

| Métrica | Valor |
|---|---|
| Unitários (pytest) | 63 passando (~0,1 s) |
| BDD (behave) | 3 features / 3 cenários / 17 steps (~0,003 s) |
| Cláusulas de erro | 6/6 obrigatórias + 9 adicionais |
| Determinismo | total (sem `random`; relógio virtual; séries roteirizadas) |
| Cenários térmicos em CI | simulados (PLAN §6.3 — nenhum hardware a 85 °C) |
