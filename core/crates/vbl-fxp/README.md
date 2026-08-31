# vbl-fxp

[![crates.io](https://img.shields.io/crates/v/vbl-fxp.svg)](https://crates.io/crates/vbl-fxp)
[![docs.rs](https://docs.rs/vbl-fxp/badge.svg)](https://docs.rs/vbl-fxp)
[![Licença](https://img.shields.io/crates/l/vbl-fxp.svg)](https://github.com/0b3st4f3r4/verbo)

**Flux Protocol (FXP)** da VerboLang: a camada única de I/O que unifica
sensores (entrada) e atores (saída) — FORMAL.md §4.4/§6 — consumida pelo
runtime via trait `vbl_runtime::Fxp`.

## Camadas

- `schema` — codec da mensagem v1 (serialização sem perda, little-endian,
  ack/seq; docs/FXP-SCHEMA-v1.md);
- `registry` — registro de dispositivos com aliases, modos
  real/simulado/híbrido e política de fallback (§4.3);
- `drivers` — backends reais (`thermal_zone`, RAPL, hwmon PWM, LED) e a fonte
  de atenção simulada (obrigatória em CI);
- `queue` — fila de comandos com prioridade (`subvert` = máxima), timeout e
  retry;
- `transport` — frames v1 sobre in-process / Unix socket / TCP com ack;
- `bus` — o `FxpBus` que roteia leituras e atos por modo de operação com
  honestidade de dados (§4.7): dispositivo inacessível em modo real ⇒
  condição não avaliada + registro no Caderno, nunca dado falsificado.

O simulador determinístico do runtime (`vbl-runtime::sim`) é o backend do modo
`simulado` — paridade bit a bit com a Etapa 2.

## Uso

O barramento é configurável por arquivo de registro e usado pelo
[`vbl-cli`](https://crates.io/crates/vbl-cli) (`--fxp-config`). O schema v1 é
estável para clientes externos (Python, embarcados) — veja
`docs/FXP-SCHEMA-v1.md` no
[repositório](https://github.com/0b3st4f3r4/verbo).

## Estado

Pré-alpha (linha `v2027.0`, fase de pesquisa). Licença: GPL-3.0-only.
