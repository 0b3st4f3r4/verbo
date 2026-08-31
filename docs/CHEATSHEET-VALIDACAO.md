# CHEATSHEET-VALIDACAO.md — validação do cheat sheet (PLAN §7)

- Data: 2026-08-30T21:35:59
- Endpoint: `http://127.0.0.1:8000/v1` · modelo: `qwen3-4b`
- Banco: 20 prompts × 3 execuções
- Verificador sintático: `tests/vlcheck.py` (mini-validador dedicado)
- Resultado: **44/60 = 73.3%** — REPROVADO (limiar ≥ 90%)

| Prompt | Exec | Veredito | Motivos |
|---|---|---|---|
| S01 | 1 | ❌ | rubrica: faltou ['attention < 30'] |
| S01 | 2 | ❌ | rubrica: faltou ['attention < 30'] |
| S01 | 3 | ❌ | rubrica: faltou ['attention < 30'] |
| S02 | 1 | ✅ | — |
| S02 | 2 | ✅ | — |
| S02 | 3 | ✅ | — |
| S03 | 1 | ✅ | — |
| S03 | 2 | ✅ | — |
| S03 | 3 | ✅ | — |
| S04 | 1 | ✅ | — |
| S04 | 2 | ✅ | — |
| S04 | 3 | ✅ | — |
| S05 | 1 | ❌ | sintaxe: atributo_duplicado L5 (atributo 'maintenance_deadline' repetido na forma 'TarefaImportante') |
| S05 | 2 | ❌ | sintaxe: atributo_duplicado L5 (atributo 'maintenance_deadline' repetido na forma 'TarefaImportante') |
| S05 | 3 | ❌ | sintaxe: atributo_duplicado L5 (atributo 'maintenance_deadline' repetido na forma 'TarefaImportante') |
| S06 | 1 | ✅ | — |
| S06 | 2 | ✅ | — |
| S06 | 3 | ✅ | — |
| S07 | 1 | ❌ | sintaxe: virgula_final L5 (vírgula final antes de '}}') |
| S07 | 2 | ❌ | sintaxe: virgula_final L6 (vírgula final antes de '}}') |
| S07 | 3 | ❌ | sintaxe: virgula_final L5 (vírgula final antes de '}}') |
| S08 | 1 | ✅ | — |
| S08 | 2 | ✅ | — |
| S08 | 3 | ✅ | — |
| S09 | 1 | ✅ | — |
| S09 | 2 | ✅ | — |
| S09 | 3 | ✅ | — |
| S10 | 1 | ❌ | sintaxe: acao_desconhecida L19 (ação desconhecida na action_list); acao_desconhecida L24 (ação desconhecida na action_list); regra_mal_formada L24 (regra deve começar com 'when'); regra_mal_formada L25 (regra deve começar com 'when') |
| S10 | 2 | ✅ | — |
| S10 | 3 | ❌ | sintaxe: acao_desconhecida L25 (ação desconhecida na action_list) |
| M01 | 1 | ✅ | — |
| M01 | 2 | ✅ | — |
| M01 | 3 | ✅ | — |
| M02 | 1 | ✅ | — |
| M02 | 2 | ✅ | — |
| M02 | 3 | ✅ | — |
| M03 | 1 | ❌ | rubrica: faltou [['falsa', 'falso disparo', 'disparo falso', 'disparos falsos', 'falsamente']] |
| M03 | 2 | ❌ | rubrica: faltou [['falsa', 'falso disparo', 'disparo falso', 'disparos falsos', 'falsamente']] |
| M03 | 3 | ✅ | — |
| M04 | 1 | ✅ | — |
| M04 | 2 | ✅ | — |
| M04 | 3 | ✅ | — |
| M05 | 1 | ✅ | — |
| M05 | 2 | ✅ | — |
| M05 | 3 | ✅ | — |
| M06 | 1 | ✅ | — |
| M06 | 2 | ✅ | — |
| M06 | 3 | ✅ | — |
| M07 | 1 | ✅ | — |
| M07 | 2 | ✅ | — |
| M07 | 3 | ✅ | — |
| M08 | 1 | ❌ | rubrica: faltou [['não são mescladas', 'não mescla']] |
| M08 | 2 | ❌ | rubrica: faltou [['não são mescladas', 'não mescla']] |
| M08 | 3 | ❌ | rubrica: faltou [['não são mescladas', 'não mescla']] |
| M09 | 1 | ✅ | — |
| M09 | 2 | ✅ | — |
| M09 | 3 | ✅ | — |
| M10 | 1 | ✅ | — |
| M10 | 2 | ✅ | — |
| M10 | 3 | ✅ | — |

> Gerado por `scripts/validate_cheatsheet.py` — não editar à mão.
