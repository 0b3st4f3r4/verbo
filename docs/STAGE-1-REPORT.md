# STAGE-1-REPORT.md — Pesquisa & Arquitetura de Escopo via Código (TDD/BDD/E2E)

| | |
|---|---|
| **Status** | Concluída — critérios de "Done" do AGENTS.md §2.2 atendidos |
| **Data** | 30/08/2026 |
| **Runner de CI (entregável §1.3 a)** | **GitHub Actions** (`.github/workflows/ci.yml`) + alvo local `make test` — a mesma suíte nos dois lugares |
| **Decisão de linguagem (entregável §1.3 d)** | **Rust** — registrada em [`docs/ADR-001-linguagem-nucleo.md`](ADR-001-linguagem-nucleo.md) com orçamentos reancorados |

---

## 1. Escopo entregue (PLAN §1.1–§1.3)

| Item | Entrega |
|---|---|
| §1.1 BDD (Gherkin) | Os 3 cenários canônicos em `tests/features/*.feature` (`# language: pt`), rodando com mocks via **behave**: 3 features, 3 cenários, **17 steps passando** |
| §1.2 TDD unitário | 63 testes pytest em `tests/unit/`: finitude, falha controlada de sensores, comandos a atores, ordem do tick, integridade do Caderno e o validador de superfície |
| §1.3 a — CI | GitHub Actions: `make check` + pytest + behave a cada push/PR; manual: `make validate-cheatsheet` (exige o LLM local) |
| §1.3 b — fronteira mock × simulador | `tests/fxp_sim/mocks.py` (**MockFXP** — dicionários em processo, sem schema binário) × `tests/fxp_sim/simulator.py` (**FXPSimulator** — §6.5: séries roteirizadas, injeção de falhas, fallback no registro, efeitos físicos determinísticos). O schema binário v1 fica para a Etapa 3 (PLAN §3.5) |
| §1.3 c — banco de 20 prompts | `docs/CHEATSHEET-PROMPTS.yaml` (S01–S10 sintaxe, M01–M10 semântica) + `scripts/validate_cheatsheet.py` (3 execuções por prompt, verificador `tests/vlcheck.py`, relatório versionado em `docs/CHEATSHEET-VALIDATION.md`, limiar ≥ 90 %) |
| §1.3 d — decisão Rust × C | ADR-001: **Rust**, com orçamentos de memória/latência reancorados e método de medição |

## 2. Arquitetura da suíte

```
tests/
├── conftest.py               fixtures: Caderno isolado, simulador, engine com injeções
├── vlcheck.py                validador de superfície .vl (mini-validador do PLAN §7)
├── fxp_sim/
│   ├── blueprint.py          carrega o protótipo como SUT (importlib)
│   ├── support.py            ConsultaCaderno (asserções sobre eventos)
│   ├── simulator.py          FXPSimulator determinístico (PLAN §6.5) + registro mínimo FORMAL §6
│   ├── mocks.py              MockFXP — fronteira em processo sem modelo físico
│   ├── ir.py                 IR dos programas (builders) + parser de durações
│   ├── loader.py             IR -> runtime (mock do front-end; parser real: Etapa 2)
│   └── contract.py           validador estrutural do IR (cláusulas de erro da FORMAL §3)
├── unit/                     63 testes (TDD — PLAN §1.2 + cláusulas de erro)
└── features/                 3 cenários BDD (Gherkin pt-BR) + environment + steps
```

**SUT:** o protótipo Python (`prototype/verbolang-complete-blueprint.py`) é a
especificação executável; o runtime recebe o FXP e o passo do relógio virtual
**injetados** (FORMAL §4.2 — determinismo total, sem `random` nos testes). Na
Etapa 2 os mesmos `.feature` e a matriz de cláusulas reancoram a implementação
Rust por adaptador.

**Camadas:** os testes unitários cobrem unidades (runtime + contrato); os
cenários BDD são o E2E **sobre mocks** — exercitam engine + FXP simulado +
Caderno de ponta a ponta em um único fluxo (Caso 1 atravessa leitura de
sensor → regra → reclassificação → persistência → trilha de auditoria). O E2E
com FXP real/hardware é a Etapa 4 (PLAN §4.2).

## 3. Critério "Done" — ≥ 1 teste por cláusula de erro (AGENTS §2.2)

| Cláusula de erro | Base | Testes |
|---|---|---|
| Sensor ausente | FORMAL §4.7 | `test_sensor_failures.py::test_sensor_ausente_nao_avalia_condicao_nem_dispara`, `::test_sensor_ausente_nao_e_tratado_como_zero`; `test_error_clauses.py::test_sensor_nao_registrado_detectado_no_ir` |
| Ator inexistente | FORMAL §4.3 | `test_actors.py::test_ator_inexistente_rejeitado_com_registro`; `test_error_clauses.py::test_ator_nao_registrado_detectado_no_ir` |
| Valor fora de limite | FORMAL §4.3 | `test_actors.py::test_valor_abaixo_do_minimo_rejeitado_sem_envio`, `::test_valor_acima_do_safety_limit_rejeitado`, `::test_valores_fora_de_limite_rejeitados[*]`, `::test_rejeicao_nao_dissolve_a_forma` |
| Forma sem `value`/`horizon` | FORMAL §3 · Lei 1 | `test_error_clauses.py::test_forma_sem_value_rejeitada_no_texto`, `::test_forma_sem_horizon_rejeitada_no_texto`, `::test_value_antes_de_horizon_e_exigido_no_texto`, `::test_forma_sem_value_ou_horizon_rejeitada_no_ir` |
| Review órfã/duplicada | FORMAL §3 | `test_error_clauses.py::test_review_orfa_rejeitada_no_texto`, `::test_review_duplicada_rejeitada_no_texto`, `::test_review_orfa_rejeitada_no_ir`, `::test_review_duplicada_rejeitada_no_ir` |
| `keep` de forma inexistente | AGENTS §2.2 | `test_error_clauses.py::test_keep_de_forma_inexistente_rejeitado_no_texto`, `::test_keep_de_forma_inexistente_rejeitado_no_ir`, `::test_keep_de_forma_dissolvida_registrado_em_runtime` |

Cláusulas adicionais pinadas: `maintenance_deadline` obrigatório em
`nonequilibrium`; `cost_bytes`/`exchange_mode` por conjugação;
`source_path` exclusivamente simbólico; unidade do threshold incompatível com
a grandeza do sensor; duração inválida; string > 256 B; ação/operador/
atributo desconhecidos; vírgula final; reclassify para `nonequilibrium` sem
deadline declarado (erro de runtime registrado).

## 4. Matriz de rastreabilidade (spec → teste)

### FORMAL §4.1 — Formas e conjugações
| Semântica | Teste |
|---|---|
| `event` expira no `horizon` (fim tipificado) | `test_finitude.py::test_event_expira_no_horizon_com_fim_tipificado` |
| `horizon` ABSOLUTO — reclassificação não renova | `test_finitude.py::test_horizon_e_absoluto_reclassificacao_nao_renova` |
| `equilibrium` também expira por `horizon` | `test_finitude.py::test_equilibrium_tambem_expira_por_horizon` |
| Manutenção implícita com regra ativa | `test_finitude.py::test_nonequilibrium_com_regra_ativa_tem_manutencao_implicita` |
| Colapso no primeiro vencimento sem manutenção | `test_finitude.py::test_nonequilibrium_sem_manutencao_colapsa_no_primeiro_vencimento` |
| `keep()` explícito renova o prazo | `test_finitude.py::test_keep_manual_renova_o_prazo` |
| Fim único por tick (regra antes dos prazos) | `test_finitude.py::test_forma_termina_uma_unicamente_por_tick` |
| Persistência `.vl` canônico + SHA-256 | BDD Caso 1 (steps 4–5); `test_vlcheck.py::test_programa_persistido_pelo_runtime_valida` |
| `reclassify_as_nonequilibrium` sem deadline declarado | `test_error_clauses.py::test_reclassify_para_nonequilibrium_sem_deadline_declarado` (e variante legal `::..._com_deadline_declarado`) |
| Ação sobre forma já dissolvida → `review_after_dissolution` | `test_error_clauses.py::test_keep_de_forma_dissolvida_registrado_em_runtime` (guarda do runtime; evento testado via contrato do tick) |

### FORMAL §4.2 — Tick
| Semântica | Teste |
|---|---|
| Regras na ordem declarada, antes dos prazos | `test_tick.py::test_regras_sao_avaliadas_na_ordem_declarada`, `test_finitude.py::test_forma_termina_uma_unicamente_por_tick` |
| `review_short_circuit` sem revogar atuações | `test_tick.py::test_review_short_circuit_apos_dissolucao` |
| Partilha igual P/N no vazamento | `test_tick.py::test_partilha_igual_da_potencia_global` |
| Relógio virtual injetável | todos os testes (engine com `tick_seconds`; FXP simulado avança o mundo) |
| Cadeia SHA-256 à prova de adulteração | `test_tick.py::test_cadeia_sha256_detecta_adulteracao`, `::test_exportacao_jsonl_reproduz_a_cadeia` |

### FORMAL §4.3/§4.5/§4.6 — Atores, `subvert`, `notify_shutdown`
| Semântica | Teste |
|---|---|
| Mensagem FXP serializada e entregue ao ator correto | `test_actors.py::test_act_e_serializado_e_entregue_ao_ator_correto`; BDD Caso 2 |
| Limites inclusivos (min/max/safety) | `test_actors.py::test_limites_sao_inclusivos` |
| `subvert`: valor poético + dissolução no mesmo tick + `act` não cancelado | BDD Caso 2 (todos os steps); `test_tick.py::test_subvert_nao_cancela_act_da_mesma_regra`, `::test_atuacao_despachada_nao_e_revogada_pelo_subvert` |
| Fallback: política do registro do FXP + trilha no Caderno | BDD Caso 3; `test_actors.py::test_fallback_executado_quando_primario_nao_responde`, `::test_fallback_esgotado_registra_alerta` |
| `notify_shutdown`: não dissolve, não interrompe | `test_tick.py::test_notify_shutdown_nao_dissolve_nem_interrompe` |

### FORMAL §4.7 — Falha de sensor
`test_sensor_failures.py` (6 testes): `0.0` é leitura válida que dispara
regras; sensor ausente/inacessível ⇒ condição não avaliada + alerta, sem
disparo falso; forma sem `source_path` não gera leitura.

### PLAN §1.2 — Asserções exigidas
| Asserção | Teste |
|---|---|
| Finitude (`event` expira no `horizon`) | `test_finitude.py` |
| (i) `0.0` válido | `test_sensor_failures.py::test_leitura_zero_e_valida_e_dispara_regras` |
| (ii) sensor ausente/inacessível sem disparo falso | `test_sensor_failures.py` |
| (iii) desalocação na dissolução (proxy de contadores; medição real: Etapa 5) | `test_finitude.py::test_contadores_de_retencao_dentro_dos_orcamentos`; BDD Caso 1 step 7, Caso 2 step 6 |
| Comandos `act` serializados e entregues (mock) | `test_actors.py`; BDD Casos 2–3 |

### Exemplos canônicos (FORMAL §5)
Os 6 exemplos da especificação validam sem erros no `vlcheck`
(`test_vlcheck.py::test_exemplos_canonicos_da_formal_validam_sem_erros`) —
e o programa gerado pela persistência é reparseável
(`::test_programa_persistido_pelo_runtime_valida`).

## 5. Interpretações registradas (lacunas da spec resolvidas em teste)

Estes pontos da FORMAL são ambíguos ou incompletos; a suíte fixa a leitura
adotada, sujeita a veto do AD:

1. **"0 bytes retidos em heap" (BDD Caso 1):** a forma reclassificada
   permanece como `equilibrium` — que **vive em suporte não volátil**
   (FORMAL §4.1). Interpretação: o estado **laborativo** é integralmente
   liberado (contadores `labor_registry` → 0); o que permanece é a estrutura
   persistida em disco, dentro do orçamento de retenção da conjugação.
2. **`reclassify_as_nonequilibrium` "sem deadline declarado":** a forma
   carrega o último `maintenance_deadline` **declarado** em alguma conjugação
   (NEQ → EQ → NEQ é legal com o prazo original); nunca declarou ⇒ erro de
   runtime registrado e a forma permanece.
3. **"Colapsa no primeiro vencimento":** vencimento = tempo desde a última
   manutenção **estritamente maior** que o `maintenance_deadline` (o tick no
   limite exato ainda sustenta). Já "expira por `horizon`" é `>=` (no limite
   exato expira) — ambos pinados em teste.
4. **Regras sobrevivem à reclassificação** (a `review` é copiada para a nova
   conjugação, como no protótipo). Consequência: uma regra pode re-disparar
   na nova conjugação. A suíte pina o comportamento atual; a SEMÂNTICA PLENA
   de renovação/conservação de regras fica para a Etapa 2 (PLAN §2.2).
5. **`VentoinhaReserva`** é extensão opcional do registro (BDD Caso 3) e
   NÃO entra no denominador da métrica de cobertura de dispositivos do
   AGENTS (só os obrigatórios do FORMAL §6 contam).

## 6. Divergências do protótipo corrigidas (alinhamento à FORMAL)

O protótipo era a referência anterior; a suíte o auditou e as seguintes
divergências foram **corrigidas no protótipo** (todas cobertas por teste):

1. FXP e passo do relógio **injetáveis** no engine (antes: instância fixa).
2. Fins de forma **tipificados** no Caderno (FORMAL §6): `dissolve_rule`,
   `dissolve_horizon`, `collapse_maintenance`, `dissolve_subvert`.
3. Eventos `review_short_circuit` e `review_after_dissolution` registrados.
4. `actor_rejected_value` na rejeição sem envio (antes: apenas print + False).
5. `keep_forma_inexistente` e `keep_ignorado` registrados (antes: silêncio).
6. **Persistência `.vl` canônico** + evento `persistencia` com caminho,
   SHA-256 e `cost_bytes` = tamanho real gravado (FORMAL §4.1) — antes era
   apenas `disk_bytes_used += 1024`.
7. **`horizon` absoluto** preservado na reclassificação — o protótipo o
   RENOVAVA (novo `creation_time`), violando a Lei 1/FORMAL §4.1.
8. `reclassify_as_nonequilibrium` sem deadline declarado recusado com erro
   registrado (antes criava NEQ com deadline fixo 3 s e horizon 60 s).
9. **Manutenção implícita** enquanto houver regra ativa (antes só o `keep`
   do `main` sustentava).
10. **Partilha igual P/N** do vazamento (antes: potência cheia para
    `PowerWatts`, 5 W fixo para as demais).
11. `subvert` sem efeito físico no runtime (o `cpu_power = 15` foi movido
    para o roteiro do mundo na demo — runtime não faz física, FORMAL §4.5).
12. `Caderno.reset()`/`event()` genérico e campos extras em alertas (kinds e
    motivos consultáveis: `sensor_nao_registrado`, `sensor_inacessível`, etc.).

## 7. Pendências assumidas (fora do escopo da Etapa 1)

| Pendência | Etapa destino |
|---|---|
| Recarregar `equilibrium` persistidas na inicialização | Etapa 2 (PLAN §2.3 — persistência com parser) |
| Schema binário v1 do FXP (Cam'n Proto/FlatBuffers), transporte local×remoto, ack/timeout | Etapa 3 (PLAN §3.5) — hoje a fronteira é em processo, sem schema |
| Aliases no registro do FXP (FORMAL §6) | Etapa 3 |
| Drivers reais + latência p95 + precisão (orçamento de erro) | Etapa 3 |
| Caderno de produção: JSONL/`log` binário assíncrono, overhead ≤ 1 % | Etapa 4 |
| Medição real de heap (`size_of` + arenas em Rust; miri/ASan; soak 24 h) | Etapas 2 e 5 — na Etapa 1 o contador de retenção é proxy determinístico |
| Validar efeitos semânticos plenos de `exchange_mode` e conservação de regras na reclassificação | Etapa 2 (PLAN §2.2) |
| Nova campanha de validação do cheat sheet até ≥ 90 % (hoje: 73,3 %) | contínua — `make validate-cheatsheet` após evolução do cheat sheet ou do modelo local |

## 8. Como executar

```bash
make setup                 # venv + requirements-dev.txt (behave, pytest, PyYAML)
make test                  # suíte completa: pytest + behave
make test-unit             # apenas unitários
make test-bdd              # apenas BDD
make check                 # checagens estáticas (shell + JS da UI)

# Validação do cheat sheet (PLAN §7) — exige o LLM local:
make serve                 # sobe o qwen3-4b via vLLM
make validate-cheatsheet   # 20 prompts × 3 execuções → docs/CHEATSHEET-VALIDATION.md
```

CI: GitHub Actions executa `make check` + pytest + behave a cada push/PR
(Python 3.13, ubuntu-latest). A validação do cheat sheet roda manualmente na
máquina local (o modelo não sobe no CI).

## 9. Métricas da suíte

| Métrica | Valor |
|---|---|
| Testes unitários (pytest) | **63 passando** em ~0,1 s |
| BDD (behave) | **3 features / 3 cenários / 17 steps passando** em ~0,003 s |
| Cláusulas de erro cobertas | 6/6 obrigatórias (AGENTS) + 9 adicionais |
| Determinismo | total (sem `random`; relógio virtual; séries roteirizadas) |
| Cenários térmicos em CI | simulados (PLAN §6.3 — nenhum hardware a 85 °C) |

## 10. Auditoria do cheat sheet (PLAN §7) — campanhas v1 e v2

O banco fixo (`docs/CHEATSHEET-PROMPTS.yaml`) foi executado contra o modelo
local **qwen3-4b** (vLLM, com o cheat sheet injetado como prompt de sistema),
3 execuções por prompt — 60 respostas por campanha.

### Campanha v1 — 45 % (REPROVADA) e triagem

A taxa baixa **não** foi aceita de imediato: cada falha foi triada em (a)
erro real do modelo, (b) lacuna do cheat sheet ou (c) defeito de rubrica/prompt.

| Causa | Evidências (v1) | Ação corretiva |
|---|---|---|
| **Lacuna do cheat sheet** — a FORMAL exige, o cheat sheet não ensinava | M02 (alerta de falha de I/O não registrado no Caderno), M06 (não há retorno a `event`), M08 (review órfã/duplicada é erro de compilação), M09 (`actor_rejected_value` não nomeado) | 4 adições pontuais em `docs/VBL-CHEATSHEET.md` (transições legais, erro de review, alerta de I/O, kind da rejeição) — auditadas contra a FORMAL §3/§4 |
| **Rubrica de formatação exata** | S02 exigia prosa explicando ausência de `maintenance_deadline`; S05 exigia linhas com espaçamento exato | Âncoras estruturais + itens negativos (`!`) avaliados sobre o bloco de código |
| **Prompt ambíguo** | S05/S08 pediam apenas fragmento → `keep` sem forma declarada e reviews órfãs eram **esperados** (o validador estava certo; o prompt, não) | Prompts passam a exigir programa completo |
| **Erro real do modelo** | S01/S06 `cost_bytes` em `nonequilibrium` (FORMAL §3); S07 vírgula final; S08 inventou formas/ações; S10 ação inexistente | Mantidas como falha legítima — é o ruído que o limiar de 90 % existe para medir |

### Campanha v2 (banco v2 + cheat sheet corrigido) — 71,7 %

Semântica **29/30** — as quatro lacunas do cheat sheet (M02, M06, M08, M09)
foram resolvidas pelas adições. As falhas residuais concentraram-se na
camada sintática.

### Campanha v3 — final (banco v3) — **73,3 % (44/60)**

Resultado completo versionado em
[`docs/CHEATSHEET-VALIDATION.md`](CHEATSHEET-VALIDATION.md); respostas brutas
para triagem do GQT em `.cheatsheet-respostas.jsonl` (ignorado pelo git).
Quebra final: **semântica 28/30** · **sintaxe 16/30**.

| Falha residual | Diagnóstico |
|---|---|
| S01 — inventa o sensor `attention_level` (o canônico é `attention`) | erro real do modelo |
| S05 — duplica `maintenance_deadline` (eco do enunciado + atributo próprio) | erro real do modelo |
| S07 — vírgula final antes de `}` (EBNF não permite) | erro real do modelo |
| S10 — inventa ações fora do conjunto canônico | erro real do modelo |
| M03/M08 — resposta correta em essência, mas sem citar a âncora exigida | rubrica rigorosa; aceitável — as âncoras derivam da FORMAL |

### Veredito

**O critério de aceite do PLAN §7 (≥ 90 %) NÃO foi atingido** — campanhas
45 % → 71,7 % → 73,3 %. O banco, o verificador e o harness estão prontos e
versionados; o gargalo é a fiabilidade do modelo local diante da gramática.
Consequência operacional (já prevista no AGENTS §5): **o modelo local não
detém conhecimento suficiente da VerboLang** — até nova campanha atingir o
limiar, toda saída dele usada como artefato passa pela validação do GQT, e a
demanda do PLAN §7 segue "em atendimento".
