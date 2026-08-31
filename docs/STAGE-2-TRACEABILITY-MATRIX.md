# Etapa 2 — Matriz de Rastreabilidade do Parser

**Critério "Done" (AGENTS.md §2.2):** 100% das produções da FORMAL §3 e ≥ 95%
das notas semânticas com ≥ 1 teste. Suíte: `core/crates/vbl-lang/tests/productions_matrix.rs`
(42 testes, todos passando) + `core/crates/vbl-lang/tests/canon_roundtrip.rs` (5).

Legenda: **Prod.** = produção EBNF (docs/FORMAL.md §3) · **Nota** = nota semântica
· **Teste(s)** = id na suíte (`producao_*`/`nota_*`/`lexico_*`/`exemplos_*`).

## 1. Produções × testes

| # | Produção EBNF | Teste(s) | Status |
|---|---------------|----------|--------|
| 1 | `program = { form_declaration \| review_declaration } [ main_block ]` | `producao_programa_formas_reviews_e_main`, `producao_programa_ordem_livre_e_main_opcional` | ✅ |
| 2 | `form_declaration = conjugation_kw identifier '{' form_body '}'` | `producao_form_declaration_estrutura`, `producao_form_declaration_duplicada` | ✅ |
| 3 | `conjugation_kw = 'event' \| 'equilibrium' \| 'nonequilibrium'` | `producao_conjugation_kw_tres_variantes` | ✅ |
| 4 | `form_body = 'value' ':' expression ',' 'horizon' ':' duration { ',' optional_attribute }` | `producao_form_body_value_e_horizon_obrigatorios_e_nesta_ordem`, `producao_form_body_virgula_final_rejeitada` | ✅ |
| 5 | `optional_attribute → 'source_path'` | `producao_optional_attribute_source_path` | ✅ |
| 6 | `optional_attribute → 'maintenance_deadline'` | `producao_optional_attribute_maintenance_deadline` | ✅ |
| 7 | `optional_attribute → 'exchange_mode'` | `producao_optional_attribute_exchange_mode` | ✅ |
| 8 | `optional_attribute → 'cost_bytes'` | `producao_optional_attribute_cost_bytes` | ✅ |
| 9 | `optional_attribute → 'currency'` | `producao_optional_attribute_currency` | ✅ |
| 10 | `optional_attribute → 'classification'` | `producao_optional_attribute_classification` | ✅ |
| 11 | `optional_attribute` (agregado: desconhecido/duplicado) | `producao_optional_attribute_desconhecido_e_duplicado` | ✅ |
| 12 | `review_declaration = 'review' identifier '{' review_rule { ',' review_rule } '}'` | `producao_review_declaration_com_regras` | ✅ |
| 13 | `review_rule = 'when' sensor_ref comparison_op threshold '->' action_list` | `producao_sensor_ref_identificador_ou_string` (mal-formada), `nota_regra_sem_seta_ou_com_estrutura_quebrada` | ✅ |
| 14 | `sensor_ref = identifier \| string` | `producao_sensor_ref_identificador_ou_string` | ✅ |
| 15 | `comparison_op = '<' \| '>' \| '<=' \| '>=' \| '==' \| '!='` | `producao_comparison_op_seis_operadores` | ✅ |
| 16 | `threshold = number \| percentage \| physical_quantity` | `producao_threshold_numero_porcentagem_e_grandeza_fisica` | ✅ |
| 17 | `action_list = action { ',' action }` | `producao_action_list_multiplas_acoes_na_ordem` | ✅ |
| 18 | `action` — 6 variantes (dissolve, subvert, reclassify×2, notify_shutdown, act) | `producao_action_seis_variantes_na_ordem_declarada` | ✅ |
| 19 | `action → 'act' '(' actor_name ',' expression ')'` | `producao_action_list_multiplas_acoes_na_ordem` (variantes `Act`) | ✅ |
| 20 | `main_block = 'main' '{' statement { ',' statement } '}'` | `producao_main_block_keep_act_every`, `producao_programa_main_deve_ser_ultimo`, `producao_programa_main_duplicado_rejeitado` | ✅ |
| 21 | `statement → 'keep' '(' identifier ')'` | `producao_main_statement_desconhecido_e_keep_inexistente` | ✅ |
| 22 | `statement → 'act' '(' actor_name ',' expression ')'` | `producao_main_block_keep_act_every` | ✅ |
| 23 | `statement → 'every' duration '{' statement { ',' statement } '}'` | `producao_main_block_keep_act_every` (aninhamento), `producao_statement_every_muito_profundo_e_rejeitado` | ✅ |
| 24 | `expression = string \| number \| identifier` | `nota_lei1_value_e_opaco_ao_runtime` | ✅ |
| 25 | `duration = number time_unit` | `producao_duration_unidades_de_tempo_e_decimais` | ✅ |
| 26 | `time_unit = 's' \| 'ms' \| 'us' \| 'ns'` | `producao_duration_unidades_de_tempo_e_decimais` | ✅ |
| 27 | `number = integer \| decimal` | `producao_number_inteiro_e_decimal` | ✅ |
| 28 | `percentage = number '%'` | `producao_threshold_numero_porcentagem_e_grandeza_fisica` | ✅ |

**Produções da FORMAL §3: 28/28 cobertas = 100%.**

## 2. Notas semânticas × testes

| # | Nota semântica (FORMAL §2/§3) | Teste(s) | Status |
|---|-------------------------------|----------|--------|
| 1 | `value` é opaco ao runtime (Lei 1) — expression com string/número/identificador | `nota_lei1_value_e_opaco_ao_runtime` | ✅ |
| 2 | `source_path` exclusivamente simbólico (nunca caminho de SO) | `nota_source_path_exclusivamente_simbolico` | ✅ |
| 3 | `maintenance_deadline` obrigatório apenas em `nonequilibrium` | `nota_maintenance_deadline_obrigatorio_em_nonequilibrium` | ✅ |
| 4 | `exchange_mode` apenas em `nonequilibrium` | `nota_exchange_mode_apenas_nonequilibrium` | ✅ |
| 5 | `cost_bytes` apenas em `equilibrium` (inteiro) | `nota_cost_bytes_apenas_equilibrium` | ✅ |
| 6 | `currency` herda padrão da conjugação (CpuCycles/DiskBytes/PowerWatts) | `nota_currency_herda_padrao_da_conjugacao` | ✅ |
| 7 | `review` órfã ou duplicada é erro de compilação | `nota_review_orfa_e_duplicada_sao_erros_de_compilacao` | ✅ |
| 8 | regra sem `->` ou com estrutura quebrada | `nota_regra_sem_seta_ou_com_estrutura_quebrada` | ✅ |
| 9 | unidade do `threshold` é capturada na AST para validação contra o registro do FXP (validada em runtime — `unidade_ausente`/`unidade_incompativel`, testes em `vbl-runtime/tests/transition.rs` via loader) | `nota_threshold_unidade_e_capturada_para_validacao_no_registro` | ✅ |

**Notas semânticas com ≥ 1 teste: 9/9 = 100% (meta ≥ 95%).**

## 3. Cláusulas de erro (FORMAL §2/§3) × códigos de diagnóstico

Cada cláusula de erro da FORMAL tem um código canônico e ≥ 1 teste:

| Código de diagnóstico | Teste(s) |
|------------------------|----------|
| `lexema_invalido` (com linha/coluna) | `lexico_lexema_invalido_com_linha_e_coluna`, `producao_comparison_op_seis_operadores`, `producao_number_inteiro_e_decimal` |
| `string_muito_longa` (limite 256 bytes) | `lexico_strings_escapes_e_limite_de_256_bytes` |
| `string_nao_terminada` | `lexico_strings_escapes_e_limite_de_256_bytes` |
| `comentario_nao_fechado` | `lexico_comentarios_de_linha_e_bloco` |
| `topo_invalido` | `producao_programa_topo_invalido` |
| `main_deve_ser_ultimo` | `producao_programa_main_deve_ser_ultimo` |
| `main_duplicado` | `producao_programa_main_duplicado_rejeitado` |
| `estrutura_forma` | `producao_form_declaration_estrutura` |
| `bloco_nao_fechado` | `producao_form_declaration_estrutura` |
| `value_obrigatorio` | `producao_form_body_value_e_horizon_obrigatorios_e_nesta_ordem` |
| `horizon_obrigatorio` | idem |
| `ordem_value_horizon` | idem |
| `virgula_final` | `producao_form_body_virgula_final_rejeitada` |
| `maintenance_deadline_ausente` | `nota_maintenance_deadline_obrigatorio_em_nonequilibrium` |
| `atributo_nao_aplicavel` | `nota_maintenance_deadline…`, `nota_exchange_mode…`, `nota_cost_bytes…` |
| `atributo_desconhecido` | `producao_optional_attribute_desconhecido_e_duplicado` |
| `atributo_duplicado` | idem |
| `duracao_invalida` | `producao_duration_unidades_de_tempo_e_decimais` |
| `cost_bytes_inteiro` | `producao_optional_attribute_cost_bytes` |
| `source_path_nao_simbolico` | `nota_source_path_exclusivamente_simbolico` |
| `review_orfa` | `nota_review_orfa_e_duplicada_sao_erros_de_compilacao` |
| `review_duplicada` | idem |
| `forma_duplicada` | `producao_form_declaration_duplicada` |
| `keep_forma_inexistente` | `producao_main_statement_desconhecido_e_keep_inexistente` (parser) + `keep_forma_inexistente` em runtime (`vbl-runtime/tests/transition.rs` via `main`) |
| `regra_mal_formada` | `producao_sensor_ref_identificador_ou_string`, `nota_threshold_unidade…` |
| `operador_invalido` | `producao_comparison_op_seis_operadores` |
| `acao_desconhecida` | `producao_action_desconhecida_rejeitada` |
| `statement_desconhecido` | `producao_main_statement_desconhecido_e_keep_inexistente` |
| `every_muito_profundo` (profundidade máx. 8) | `producao_statement_every_muito_profundo_e_rejeitado` (8 níveis aceitos, 9 rejeitado) |

## 4. Cobertura de runtime dos códigos (camada loader/engine)

Códigos de carga/runtime validados contra o registro do FXP
(`vbl-runtime/tests/transition.rs`):

| Código | Teste(s) em `transition.rs` |
|--------|----------------------------|
| `sensor_nao_registrado` (alerta `motivo` em §4.7) | `sensor_ausente_nao_avalia_condicao_nem_dispara`, `sensor_ausente_nao_e_tratado_como_zero` |
| `sensor_inacessivel` (alerta `motivo`) | `sensor_registrado_inacessivel_segue_a_mesma_regra` |
| `unidade_ausente` / `unidade_incompativel` (diagnóstico de carga) | `validar` (loader) — validação exercitada via `carregar` em todos os testes; cláusula §1.2 do AGENTS (grandeza × unidade) |
| `ator_nao_registrado` (diagnóstico de carga) / `ator_inexistente` (evento) | `ator_inexistente_rejeitado_com_registro` |
| `ator_indisponivel` (evento) | `fallback_do_registro_e_acionado_quando_primario_falha` |
| `fallback_executado` | idem |
| `actor_rejected_value` (limites inclusivos) | `valor_abaixo_do_minimo_rejeitado_sem_envio`, `valor_acima_do_safety_limit_rejeitado`, `ator_fora_do_maximo_rejeitado_sem_envio`, `limites_sao_inclusivos` |
| `reclassify_sem_deadline` (runtime, FORMAL §3) | `reclassify_para_nonequilibrium_sem_deadline_e_erro_registrado` |
| `keep_forma_inexistente` / `keep_ignorado` (runtime) | exercitados via `main` — `keep_manual_renova_o_prazo`, `forma_termina_uma_unicamente_por_tick` |

## 5. Exemplos canônicos

`exemplos_canonicos_da_formal_validam` — os exemplos §5 da FORMAL (PensarLivre,
TradingEspeculativo e demais) validam sem diagnóstico. Os mesmos programas
estão em `examples/*.vl` e são executados ponta a ponta pelo CLI `vbl`
(`vbl check`, `vbl run`).

## 6. Como reproduzir

```bash
make rust-test     # cargo test — matriz (41) + canon (5) + transição (36)
make rust-lint     # clippy --workspace --all-targets -- -D warnings
make rust-asan     # AddressSanitizer (vazamentos)
make rust-bench    # criterion — orçamentos de latência
```
