# vbl-cli

[![crates.io](https://img.shields.io/crates/v/vbl-cli.svg)](https://crates.io/crates/vbl-cli)
[![docs.rs](https://docs.rs/vbl-cli/badge.svg)](https://docs.rs/vbl-cli)
[![Licença](https://img.shields.io/crates/l/vbl-cli.svg)](https://github.com/0b3st4f3r4/verbo)

`vbl` — interpretador de console para programas `.vl` da **VerboLang**:
valida, executa e audita formas que dissipam energia enquanto existem.

## Instalação

```sh
cargo install vbl-cli --locked
```

## Uso

```sh
# valida o programa (parser + registro FXP) — diagnósticos com linha/coluna
vbl check exemplo.vl

# executa com relógio virtual (padrão) ou tempo real (--real-ms)
vbl run exemplo.vl
vbl run exemplo.vl --real-ms

# audita o registro FXP do host (dispositivo × modo × rota × latência)
vbl fxp-probe

# verificação EXTERNA do log do Caderno: recomputa a cadeia SHA-256
vbl ledger-verify caderno.vcad
```

Exemplo mínimo (`exemplo.vl`):

```verbolang
nonequilibrium FreeThinking {
    value: "consciencia_antineoliberal_ativa",
    horizon: 60s,
    source_path: "attention",
    maintenance_deadline: 3s,
    exchange_mode: "cooperation"
}

review FreeThinking {
    when attention < 30% -> reclassify_as_equilibrium
}
```

Sem `--fxp-config`, o backend é o simulador determinístico em processo
(padrão para estudo e CI). Com um arquivo de registro, o `run` sobe o
barramento FXP real — `thermal_zone`, RAPL, hwmon PWM, LED — com honestidade
de dados: dispositivo inacessível ⇒ condição não avaliada e registro no
Caderno, nunca leitura falsificada.

Crates irmãos: [`vbl-lang`](https://crates.io/crates/vbl-lang),
[`vbl-runtime`](https://crates.io/crates/vbl-runtime) e
[`vbl-fxp`](https://crates.io/crates/vbl-fxp).

## Estado

Pré-alpha (linha `v2027.0`, fase de pesquisa). Licença: GPL-3.0-only.
