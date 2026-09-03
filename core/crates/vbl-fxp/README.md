# vbl-fxp

[![crates.io](https://img.shields.io/crates/v/vbl-fxp.svg)](https://crates.io/crates/vbl-fxp)
[![docs.rs](https://docs.rs/vbl-fxp/badge.svg)](https://docs.rs/vbl-fxp)
[![Licença](https://img.shields.io/crates/l/vbl-fxp.svg)](https://github.com/0b3st4f3r4/verbo)

**Flux Protocol (FXP)** da VerboLang: a camada única de I/O que unifica
sensores (entrada) e atores (saída) — FORMAL.md §4.4/§6 — consumida pelo
runtime via trait `vbl_runtime::Fxp`.

## Camadas

- `schema` — codec da mensagem v1.4 (serialização sem perda, little-endian,
  ack/seq; docs/FXP-SCHEMA-v1.md): CAPS (§4.5), AUTH PSK-HMAC-SHA256 (§4.6),
  READ_BATCH (§4.7), compressão do corpo (§4.8: LZ4, LZ4+dict, zstd+dict
  treinado, zstd+dict treinado verificado — `DICT_SYNC`) e FLAG_TIMESTAMP
  (§5 — carimbo físico do fio, anotação de laboratório);
- `auth` — MAC/nonce do handshake PSK (§4.6): HMAC-SHA256 sobre
  `"FXP-AUTH1" ‖ nonce_cliente ‖ nonce_servidor`, verificação em tempo
  constante, nonces de 32 B por conexão;
- `discover` — beacon multicast `FXPD` (§4.9): anúncio periódico
  (`Announcer`) e escuta (`discover_peers`), opt-in, sem dado de sensor no
  anúncio; IPv6, SSM v4 e SSM v6 (assinatura de fonte, RFC 3678);
- `registry` — registro de dispositivos com aliases, modos
  real/simulado/híbrido, política de fallback (§4.3) e endpoint
  `discover:<identificador>`;
- `drivers` — backends reais (`thermal_zone`, RAPL, hwmon PWM, LED) e a fonte
  de atenção simulada (obrigatória em CI);
- `queue` — fila de comandos com prioridade (`subvert` = máxima), timeout e
  retry;
- `transport` — frames v1.4 sobre in-process / Unix socket / TCP / TLS 1.3
  com ack, negociação CAPS (inclusive como 0-RTT na retomada, §7), handshake
  AUTH e dicionário de compressão tipado;
- `peer` — `PeerServer`, servidor de referência do schema (máquina de estados
  AUTH → CAPS → trabalho; serve Unix/TCP, anuncia recursos por flag no
  `vbl fxpd`);
- `bus` — o `FxpBus` que roteia leituras e atos por modo de operação com
  honestidade de dados (§4.7): dispositivo inacessível em modo real ⇒
  condição não avaliada + registro no Caderno, nunca dado falsificado;
  lote (§4.7) e compressão (§4.8) são opt-in por config e degradam para
  v1.0 puro com peer antigo (evento `fxp_peer_v1` no Caderno); TLS com
  pin único ou multi-pin (`@sha256:HEX[,HEX2,…]` — rotação com
  sobreposição), TOFU aprendiz (`@tofu`) ou estrito (`@tofu-estrito`,
  v1.4 — allow-list que nunca aprende), store `TofuStore` JSON atômico;
- `tls` — TLS 1.3 (rustls/`ring`) com pinning SHA-256 (multi-pin), TOFU
  aprendiz e estrito (falha fechada na divergência/ausência), cache de
  `ClientConfig` (resumo de sessão) e 0-RTT para o `CAPS` (§7);
- `sessoes` — cache de sessões TLS do SERVIDOR em disco (v1.4 §7,
  `--tls-sessions`): a retomada (com 0-RTT) sobrevive ao renascimento do
  processo — write-through atômico `0600`, evicção por idade, teto 1024;

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
