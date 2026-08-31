# CHEATSHEET-VALIDATION.md — validação do cheat sheet (PLAN §7)

- Data: 2026-08-31T14:22:45
- Endpoint: `http://127.0.0.1:8000/v1` · modelo: `qwen3-4b`
- Banco: 20 prompts × 3 execuções
- Verificador sintático: `tests/vlcheck.py` (mini-validador dedicado)
- Resultado: **20/20 = 100.0%** — ACEITO (limiar ≥ 90%)

| Prompt | Exec | Veredito | Motivos |
|---|---|---|---|
| S01 | 1 | ✅ | — |
| S02 | 1 | ✅ | — |
| S03 | 1 | ✅ | — |
| S04 | 1 | ✅ | — |
| S05 | 1 | ✅ | — |
| S06 | 1 | ✅ | — |
| S07 | 1 | ✅ | — |
| S08 | 1 | ✅ | — |
| S09 | 1 | ✅ | — |
| S10 | 1 | ✅ | — |
| M01 | 1 | ✅ | — |
| M02 | 1 | ✅ | — |
| M03 | 1 | ✅ | — |
| M04 | 1 | ✅ | — |
| M05 | 1 | ✅ | — |
| M06 | 1 | ✅ | — |
| M07 | 1 | ✅ | — |
| M08 | 1 | ✅ | — |
| M09 | 1 | ✅ | — |
| M10 | 1 | ✅ | — |

> Gerado por `scripts/validate_cheatsheet.py` — não editar à mão.
