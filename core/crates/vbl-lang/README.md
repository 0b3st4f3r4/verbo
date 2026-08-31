# vbl-lang

[![crates.io](https://img.shields.io/crates/v/vbl-lang.svg)](https://crates.io/crates/vbl-lang)
[![docs.rs](https://docs.rs/vbl-lang/badge.svg)](https://docs.rs/vbl-lang)
[![Licença](https://img.shields.io/crates/l/vbl-lang.svg)](https://github.com/0b3st4f3r4/verbo)

Front-end da **VerboLang**: lexer, parser e AST da linguagem de formas —
programas que dissipam energia enquanto existem e por isso carregam horizonte.

Implementa as unidades léxicas (FORMAL.md §2) e a gramática EBNF com notas
semânticas (§3). O parser devolve a AST **e** todos os diagnósticos
encontrados — programas inválidos produzem diagnósticos com código canônico,
linha e coluna, em vez de parar no primeiro erro.

## Camadas

- `token` / `lexer` — unidades léxicas da FORMAL §2;
- `ast` — estrutura do programa (formas, horizontes, `review`, `main`);
- `parser` — descida recursiva sobre a EBNF + cláusulas de erro;
- `canon` — serialização `.vl` canônica reparseável (FORMAL §4.1);
- `diag` — diagnóstico com código canônico, linha e coluna.

## Uso

```rust
use vbl_lang::parser::Parser;

let source = r#"
nonequilibrium FreeThinking {
    value: "consciencia_anteneoliberal_ativa",
    horizon: 60s,
    source_path: "attention",
    maintenance_deadline: 3s,
    exchange_mode: "cooperation"
}
"#;

let (ast, diags) = Parser::new(source).parse();
assert!(diags.is_empty());
```

## Estado

Pré-alpha (linha `v2027.0`, fase de pesquisa). A semântica canônica é a
especificação: [`docs/FORMAL.md`](https://github.com/0b3st4f3r4/verbo/blob/main/docs/FORMAL.md).

Crates irmãos: [`vbl-runtime`](https://crates.io/crates/vbl-runtime) (motor de
tick), [`vbl-fxp`](https://crates.io/crates/vbl-fxp) (I/O físico) e
[`vbl-cli`](https://crates.io/crates/vbl-cli) (interpretador `vbl`).

Licença: GPL-3.0-only.
