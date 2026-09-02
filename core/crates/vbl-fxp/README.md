# vbl-fxp

[![crates.io](https://img.shields.io/crates/v/vbl-fxp.svg)](https://crates.io/crates/vbl-fxp)
[![docs.rs](https://docs.rs/vbl-fxp/badge.svg)](https://docs.rs/vbl-fxp)
[![Licença](https://img.shields.io/crates/l/vbl-fxp.svg)](https://github.com/0b3st4f3r4/verbo)

**Flux Protocol (FXP)** da VerboLang: a camada única de I/O que unifica
sensores (entrada) e atores (saída) — FORMAL.md §4.4/§6 — consumida pelo
runtime via trait `vbl_runtime::Fxp`.

## Camadas

- `schema` — codec da mensagem v1.1 (serialização sem perda, little-endian,
  ack/seq; docs/FXP-SCHEMA-v1.md): CAPS (§4.5), AUTH PSK-HMAC-SHA256 (§4.6),
  READ_BATCH (§4.7), compressão LZ4 do corpo (§4.8) e FLAG_TIMESTAMP (§5 —
  carimbo físico do fio, anotação de laboratório);
- `auth` — MAC/nonce do handshake PSK (§4.6): HMAC-SHA256 sobre
  `"FXP-AUTH1" ‖ nonce_cliente ‖ nonce_servidor`, verificação em tempo
  constante, nonces de 32 B por conexão;
- `discover` — beacon multicast `FXPD` (§4.9): anúncio periódico
  (`Announcer`) e escuta (`discover_peers`), opt-in, sem dado de sensor no
  anúncio;
- `registry` — registro de dispositivos com aliases, modos
  real/simulado/híbrido, política de fallback (§4.3) e endpoint
  `discover:<identificador>`;
- `drivers` — backends reais (`thermal_zone`, RAPL, hwmon PWM, LED) e a fonte
  de atenção simulada (obrigatória em CI);
- `queue` — fila de comandos com prioridade (`subvert` = máxima), timeout e
  retry;
- `transport` — frames v1.1 sobre in-process / Unix socket / TCP com ack,
  negociação CAPS e handshake AUTH;
- `peer` — `PeerServer`, servidor de referência do schema (máquina de estados
  AUTH → CAPS → trabalho; serve Unix/TCP, anuncia recursos por flag no
  `vbl fxpd`);
- `bus` — o `FxpBus` que roteia leituras e atos por modo de operação com
  honestidade de dados (§4.7): dispositivo inacessível em modo real ⇒
  condição não avaliada + registro no Caderno, nunca dado falsificado;
  lote (§4.7) e compressão (§4.8) são opt-in por config e degradam para
  v1.0 puro com peer antigo (evento `fxp_peer_v1` no Caderno).

O simulador determinístico do runtime (`vbl-runtime::sim`) é o backend do modo
`simulado` — paridade bit a bit com a Etapa 2.

## Uso

O barramento é configurável por arquivo de registro e usado pelo
[`vbl-cli`](https://crates.io/crates/vbl-cli) (`--fxp-config`; o servidor de
referência é `vbl fxpd`). O fio padrão é byte a byte o do v1.0 (teste de
bytes-fixos na suíte); os recursos v1.1 são **negociados** (CAPS §4.5) e
**opt-in** — cliente e servidor antigos continuam inter operando. O schema
está em `docs/FXP-SCHEMA-v1.md` no
[repositório](https://github.com/0b3st4f3r4/verbo).

## Estado

Pré-alpha (linha `v2027.0`, fase de pesquisa). Licença: GPL-3.0-only.
