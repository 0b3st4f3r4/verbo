# vbl-runtime

[![crates.io](https://img.shields.io/crates/v/vbl-runtime.svg)](https://crates.io/crates/vbl-runtime)
[![docs.rs](https://docs.rs/vbl-runtime/badge.svg)](https://docs.rs/vbl-runtime)
[![Licença](https://img.shields.io/crates/l/vbl-runtime.svg)](https://github.com/0b3st4f3r4/verbo)

Motor de tick da **VerboLang** (FORMAL.md §4): o runtime que mantém formas
vivas, avalia `review` na ordem declarada, dissolve o que venceu o horizonte e
registra cada joule dissipado.

## Componentes

- `form` — forma ativa (horizonte absoluto, manutenção, retenção);
- `scheduler` — fila de prazos: min-heap por `horizon`/`maintenance_deadline`,
  mutação O(log N), varredura O(N + vencidos) por tick;
- `engine` — loop de tick: regras na ordem declarada, prazos depois;
- `main_interp` — bloco `main` (`keep` / `act` / `every`);
- `ledger` + `production_ledger` — Caderno termodinâmico com cadeia SHA-256;
- `fxp` + `sim` — trait de barramento de I/O e simulador determinístico
  (backend padrão quando não há hardware);
- `loader` — AST → runtime, com validação contra o registro do FXP;
- `persist` — `equilibrium` em suporte estável (`.vl` canônico + SHA-256);
- `json` — serialização JSON determinística para auditoria.

Feature opt-in `heap-audit`: auditor de contagem de heap por forma
(desenvolvimento — nunca em produção).

## Uso

Veja o interpretador pronto em
[`vbl-cli`](https://crates.io/crates/vbl-cli) (`cargo install vbl-cli`) e o
front-end [`vbl-lang`](https://crates.io/crates/vbl-lang). A semântica
canônica é a especificação:
[`docs/FORMAL.md`](https://github.com/0b3st4f3r4/verbo/blob/main/docs/FORMAL.md).

## Estado

Pré-alpha (linha `v2027.0`, fase de pesquisa). Licença: GPL-3.0-only.
