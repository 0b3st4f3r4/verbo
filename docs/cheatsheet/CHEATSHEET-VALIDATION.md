# CHEATSHEET-VALIDATION.md — validação do cheat sheet (PLAN §7)

- Data: 2026-08-31T07:02:07
- Endpoint: `http://127.0.0.1:8000/v1` · modelo: `qwen3-4b`
- Banco: 20 prompts × 3 execuções
- Verificador sintático: `tests/vlcheck.py` (mini-validador dedicado)
- Resultado: **56/60 = 93.3%** — ACEITO (limiar ≥ 90%)

| Prompt | Exec | Veredito | Motivos |
|---|---|---|---|
| S01 | 1 | ✅ | — |
| S01 | 2 | ✅ | — |
| S01 | 3 | ✅ | — |
| S02 | 1 | ✅ | — |
| S02 | 2 | ✅ | — |
| S02 | 3 | ✅ | — |
| S03 | 1 | ✅ | — |
| S03 | 2 | ✅ | — |
| S03 | 3 | ✅ | — |
| S04 | 1 | ✅ | — |
| S04 | 2 | ✅ | — |
| S04 | 3 | ✅ | — |
| S05 | 1 | ✅ | — |
| S05 | 2 | ✅ | — |
| S05 | 3 | ✅ | — |
| S06 | 1 | ✅ | — |
| S06 | 2 | ✅ | — |
| S06 | 3 | ✅ | — |
| S07 | 1 | ✅ | — |
| S07 | 2 | ✅ | — |
| S07 | 3 | ✅ | — |
| S08 | 1 | ✅ | — |
| S08 | 2 | ❌ | sintaxe: acao_desconhecida L11 (ação desconhecida na action_list); regra_mal_formada L11 (regra deve começar com 'when'); regra_mal_formada L12 (regra deve começar com 'when') |
| S08 | 3 | ❌ | sintaxe: acao_desconhecida L11 (ação desconhecida na action_list); regra_mal_formada L11 (regra deve começar com 'when'); regra_mal_formada L12 (regra deve começar com 'when') |
| S09 | 1 | ✅ | — |
| S09 | 2 | ✅ | — |
| S09 | 3 | ✅ | — |
| S10 | 1 | ✅ | — |
| S10 | 2 | ✅ | — |
| S10 | 3 | ✅ | — |
| M01 | 1 | ❌ | rubrica: faltou [['caderno', 'registrado', 'log']] |
| M01 | 2 | ✅ | — |
| M01 | 3 | ❌ | rubrica: faltou [['caderno', 'registrado', 'log']] |
| M02 | 1 | ✅ | — |
| M02 | 2 | ✅ | — |
| M02 | 3 | ✅ | — |
| M03 | 1 | ✅ | — |
| M03 | 2 | ✅ | — |
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
| M08 | 1 | ✅ | — |
| M08 | 2 | ✅ | — |
| M08 | 3 | ✅ | — |
| M09 | 1 | ✅ | — |
| M09 | 2 | ✅ | — |
| M09 | 3 | ✅ | — |
| M10 | 1 | ✅ | — |
| M10 | 2 | ✅ | — |
| M10 | 3 | ✅ | — |

> Gerado por `scripts/validate_cheatsheet.py` — não editar à mão.
