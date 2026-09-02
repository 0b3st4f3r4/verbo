# FXP v1.1 — Relatório de extensões do fio

> Janela `v2027.0.0-alpha.1` (Setembro/2026) · item 8 do
> [`PLAN.md` §8](../PLAN.md) · origem: §9 do
> [`FXP-SCHEMA-v1.md`](../FXP-SCHEMA-v1.md) — o documento do schema foi
> **promovido a v1.1** (não existe `FXP-SCHEMA-v1.1.md`; §9 lá lista como
> "implementado na v1.1" e deixa como futuro só o que de fato ficou fora).

## 1. Escopo entregue

As cinco extensões registradas como "Extensões futuras" no §9 do schema
v1.0, agora negociadas e **opt-in** — o fio padrão é byte a byte o do v1.0
(teste de bytes-fixos: `golden_wire_default_igual_ao_v1_0` em
`core/crates/vbl-fxp/tests/schema_roundtrip.rs`):

| Extensão | Contrato | Implementação | Testes |
|---|---|---|---|
| **CAPS/CAPS_OK** (§4.5) | bits LZ4=1<<0, BATCH=1<<1, TIMESTAMP=1<<2; interseção cliente∩servidor; reservados rejeitados no encode | `schema::{caps, Body::Caps/CapsOk}`, `Connection::negotiate` | `transport.rs` (4), `v11.rs` e2e |
| **AUTH PSK** (§4.6) | `AUTH_CHALLENGE`→`AUTH_RESPONSE`→`AUTH_OK`; MAC = HMAC-SHA256(`"FXP-AUTH1"` ‖ nonce_c ‖ nonce_s), nonces 32 B, tempo constante; falha **termina** a conexão (sem fallback) | `auth.rs`, `Connection::authenticate`, `PeerServer` (challenge primeiro; pré-auth só AUTH_RESPONSE) | `schema_roundtrip.rs`, `v11.rs` (chave certa/errada/ausente), `e2e.rs` CLI |
| **READ_BATCH** (§4.7) | 1..=64 nomes; resposta por item (`0=ok f64+canônico, senão READ_ERR`); item que falha **não** gera alerta — o alerta pertence à pergunta do programa | `Message::read_batch/read_batch_ok`, `FxpBus::{batch_prefetch, read_remote_batch, alvos_de_batch}` (≥2 sensores vencidos do mesmo peer) | `v11.rs` (honestidade por item), `e2e.rs` |
| **Compressão LZ4** (§4.8) | bit 6 da flag; byte reservado = id do algoritmo (1=LZ4); região < 512 B e blob que infla nunca comprimem; teto 8192 B descomprimido (guarda de bomba) | `encode_with_compression`, `compress::{THRESHOLD, ALGO_LZ4}`, decode com `decompress_into` + buffer-teto | `schema_roundtrip.rs` (roundtrip, threshold, algoritmo desconhecido, bomba) |
| **FLAG_TIMESTAMP** (§5) | bit 5; `timestamp_us: Option<u64>` após o header, antes do nome; **o Caderno permanece no relógio virtual** — o carimbo é anotação de laboratório (`wire_timestamp_of`, `fio_us` nos eventos sintéticos) | campo + `with_timestamp`, `PeerServer` carimba `now_unix_us()` quando negociado, bus captura `wire_ts` | codec + e2e (`wire_timestamp_of`) |
| **Beacon FXPD** (§4.9) | UDP `239.255.70.80:7080`, TTL 1, 2 s, opt-in; anúncio sem dado de sensor; hash informativo do registro (SHA-256 dos nomes canônicos) | `discover.rs` (`Announcer`, `discover_peers`, `registry_hash`), endpoint `discover:<id>` resolvido no `build()` | `tests/discover.rs` (loopback com skip gracioso), `v11.rs` e2e de resolução |

Mais a entrega de operação: **`vbl fxpd`** — o servidor de referência do
schema (§7), com `--serve unix:PATH|tcp:PORTA`, `--auth psk:VAR`,
`--announce ID`, `--batch`, `--timestamp`, `--compress`, `--ledger`; e no
cliente `vbl run`: `--fxp-psk-env VAR` + chaves de config
`batch_prefetch`/`compression`/`wire_timestamp`/`compress_threshold` (§6).

**Degradação v1.0**: peer sem CAPS_OK (fecha ou responde lixo) ⇒ o cliente
reconecta plano, registra `fxp_peer_v1` no Caderno e segue operando
(`e2e_peer_v1_0_degrada_com_evento_e_continua_operando`). `Timeout` na
negociação é falha honesta, não degradação.

**Governação (anti-entropia)**: dono canônico do estado de servidor é
`vbl-fxp/src/peer.rs` (`PeerServer`); `serve_unix`/`serve_tcp` continuam
canos burros, e o `vbl fxpd` é apenas plumbing. O §9 do próprio
`FXP-SCHEMA-v1.md` foi aposentado como lista de futuros — sem arquivo
duplicado de schema.

## 2. Medidas (máquina de referência: AMD Ryzen 7 7735HS, `cargo bench --quick`)

### 2.1 Lote (§4.7) — o ganho principal

Ciclo completo de atualização de 8 sensores remotos (peer real via Unix
socket, `cache_ttl` zerado para medir sempre o fio; no lote, cache de 1 s
invalidado a cada iteração):

| Bench | Tempo | RTTs |
|---|---|---|
| `fxp_v11_batch/ciclo_8_sensores_individual` | **117,4 µs** | 8 READs = 8 RTTs |
| `fxp_v11_batch/ciclo_8_sensores_lote_1rtt` | **22,3 µs** | 1 READ_BATCH + 7 acertos de cache |

**5,3× mais rápido** — e a diferença cresce com o número de sensores por
peer, pois a conexão única por *endereço* (não por dispositivo) é o que
torna o lote possível.

### 2.2 FLAG_TIMESTAMP (§5) — custo do carimbo

| Bench | Tempo |
|---|---|
| `fxp_schema_v1/encode_decode_read_ok` (plano) | 113,5 ns |
| `fxp_v11_fio/roundtrip_com_timestamp` | 119,6 ns |

**+5 ns** por roundtrip de codec (+8 bytes no fio). Custo de laboratório:
zero no Caderno (relógio virtual intocado; `tick`/`t` reservados).

### 2.3 Compressão LZ4 (§4.8) — banda, não CPU

HELLO de 60 dispositivos (nomes longos repetidos):

| Métrica | Plano | LZ4 |
|---|---|---|
| Tamanho no fio | 3 218 B | **300 B** (−90,7%) |
| `encode_decode_hello_plano` / `_lz4` | 7,9 µs | 9,2 µs |

O roundtrip comprimido custa **+17% de CPU** para **−90,7% de banda** — a
troca certa para HELLO grande em rede lenta; e o threshold de 512 B evita
pagar o preço em frames pequenos (`compressao_nao_viaja_abaixo_do_threshold`).
Lote de 64 itens no codec: 5,6 µs.

### 2.4 Handshake PSK+CAPS (§4.6) — custo do fio

| Bench | Tempo |
|---|---|
| `fxp_v11_auth/conectar_ler_plano` | 5,097 ms |
| `fxp_v11_auth/conectar_auth_caps_ler` | 5,101 ms |

**+4 µs** (challenge + HMAC + nonces + CAPS) sobre a conexão plana. A
latência total é dominada pelo *polling* de 5 ms do loop de aceitação —
comum a ambos os caminhos e alvo de otimização futura (não do v1.1).
Autenticação ≠ confidencialidade: a chave nunca trafega, mas o corpo segue
plano — confidencialidade (rustls) segue como trabalho futuro no §9 do
schema.

## 3. Cobertura de testes adicionada

- `vbl-fxp/tests/schema_roundtrip.rs`: 24 testes (+10 v1.1: opcodes, golden
  bytes v1.0, timestamp, flags derivadas, batch 1..=64 honesto, auth scheme,
  caps reservados, compressão ×3, varredura de truncamento em **todos** os
  prefixos de 13 mensagens — nunca pânico nem leitura fora do buffer — e
  roundtrip dos 9 estados de `ACT_ACK`).
- `vbl-fxp/tests/transport.rs`: +7 (negociação ok/sem capacidade/timeout,
  peer v1.0 fecha; challenge com scheme desconhecido ⇒ rejeição **tipada**
  do schema; resposta sem `AUTH_OK` ⇒ recusa honesta; frame que chega
  partido em duas escritas é remontado; servidor que some no meio do frame
  ⇒ erro honesto).
- `vbl-fxp/tests/v11.rs`: novo — 15 e2e in-crate (CAPS+batch+ts+compressão,
  auth aberta/errada/ausente, honestidade do lote, degradação v1.0,
  descoberta resolve/inacessível; HELLO/HEARTBEAT/opcodes desconhecidos;
  peer fecha sem PSK e sob lixo pré-autenticação; acks tipados
  Rejected-Min/Max/Safety que só o peer conhece; item de lote
  `nao_registrado` viajando como tag 4; resposta de lote com opcode errado
  ⇒ fechamento e alerta, nunca confiança; `act` com `Str`; `act_with_priority`
  e `invalidate_cache`).
- `vbl-fxp/tests/discover.rs`: novo — 8 (beacon roundtrip+hash, anúncio→
  escuta, dedupe, silêncio honesto, IPv6 fora do escopo ⇒ erro honesto,
  porta de grupo ocupada, ciclo de vida do `Announcer`, beacon não-UTF8;
  skip gracioso sem multicast — lição b7537d2).
- `vbl-fxp/tests/registry.rs`: +3 v1.1 (chaves globais de config com
  cláusulas de erro, endpoint `discover:` com cláusulas,
  `to_device_desc` sensor×ator).
- `vbl-cli`: 41 testes in-process (dispatch dos subcomandos, cláusulas de
  erro do `fxpd_preparar` — porta ocupada, prefixo inválido, PSK
  ausente/vazia/errada; `fxpd` real numa thread destacada; probe com rotas
  unix/tcp/discover/hwmon/rapl; `ledger-verify` com JSONL corrompida;
  `run --real-ms`) + 2 e2e fora-de-processo (`vbl fxpd` real × `vbl run`
  real: lote+ts+compress negociados com `fxp_batch` no Caderno; PSK certa
  abre, errada fecha com motivo `auth:` no Caderno).
- Bugs apanhados pelos testes durante a etapa: prefixo `length` não
  reescrito no frame comprimido; `lz4_flex::decompress` exige tamanho exato
  (troca por `decompress_into` + buffer-teto, que é a própria guarda de
  bomba); **colisão de tag no lote** — a razão 0 (`nao_registrado`) de
  §4.1 serializava como o status 0 do item, que significa "ok": o cliente
  leria um `f64` fantasma e dessincronizaria. Correção aditiva dentro do
  v1.1: razão 0 viaja como **tag 4** (§4.7 do schema atualizado; bytes
  0..=3 preservados).

## 3.1 Cobertura de linhas (portaria CI: `--fail-under-lines 95`)

**95,04%** (13 837 linhas, 687 não cobertas) — **portão verde**,
verificado com o comando exato do CI (`cargo +nightly llvm-cov --workspace
--summary-only --fail-under-lines 95`, saída 0). A campanha de fechamento
(GQT, "Prossiga com a cobertura") partiu de 93,89% e somou baterias novas em:

- `vbl-runtime/tests/production_ledger.rs` (21 → 31 testes): rodapé do
  `.vcad` (head não-UTF-8, head mentiroso, contagem de eventos mentirosa),
  frame com hash cortada, `verify_jsonl` exigindo kind/hash tolerando msg/seq
  ausentes, `jsonl_from_binary` (caminho feliz com ACTUATION, não-vcad,
  hash cortada), `verify_binary` em arquivo ausente e cláusulas de borda de
  `Json::parse` (escapes completos, objetos/arrays malformados).
- `vbl-runtime/tests/engine_clauses.rs` (novo, 4): `notify_shutdown`,
  reclassificação recusada por falta de deadline, `exchange_mode` não
  canônica ("permuta") com default `cooperation`, vazamento sem formas ativas.
- `vbl-runtime/tests/transition.rs`: `SimulatorBuilder::default` e
  `set_fallback` de ator no SIM.
- `vbl-fxp/tests/v11.rs` (17 → 24): matriz completa de descritores do HELLO
  (sensor sem limites × ator com safety), TCP recusado no barramento, peer que
  fecha após CAPS (reconexão honesta), lote de 1 no caminho v1.0, três roubos
  de lote (sem o sensor pedido, resposta trocada, peer que some), rejeição de
  limite do CLIENTE antes do fio, `update_power` com RAPL ausente, fallback
  local `FallbackExecuted`, sensor/ator desconhecidos com evento no Caderno e
  bordas do peer cru (READ_BATCH sem caps, ACT forjado, HELLO comprimido, BYE).
- `vbl-fxp/tests/transport.rs` (15 → 21): TCP com host não resolvível/porta
  recusada, unix inexistente, timeout sem resposta, frame de tamanho zero e
  dois frames chegando juntos (buffer encadeado), compressão que infla não viaja.
- `vbl-fxp/tests/schema_roundtrip.rs` (24 → 29): mensagens de erro legíveis,
  frames forjados (ACT sem nome, kind/status desconhecidos, descritor de kind
  9, nonce cortado), HELLO com 65 537 descritores, `opcode_name` dos
  opcodes de corpo vazio.
- `vbl-fxp/tests/registry.rs` (15 → 16): `ConflictingAlias`, `ChainedAlias` e
  `compress_threshold` não-inteiro.
- `vbl-lang/tests/error_clauses.rs`: atores com string em regra/main,
  duração com número e EOF, `every` sem `{` e `every_muito_profundo`.
- `vbl-cli` (args/main/e2e): posicionais extras, `--fxp-mode` inválido no run
  e no fxpd, `--ledger` no peer montado, probe em modo real com rota `auto`,
  bloqueio do export JSONL no sumário do `run`.

O resíduo de 687 linhas é defesa em profundidade inalcançável sem caos
induzido (locks envenenados, `write` falho no meio do frame, RNG ausente,
linhas-fechamento de regiões instrumentadas, `parse_args` já recusando antes
dos guards internos), documentado no inventário do GQT — nenhum
`#[coverage(off)]` foi usado.

Triagem de flake (AGENTS: 0 não triados): `truncated_file_rejected...`
falhava esporadicamente porque dois testes do MESMO binário reusavam o nome
`temp_dir("truncado")` — o `remove_dir_all` inicial de um apagava os arquivos
do outro em paralelo. Corrigido com nome único; 5 execuções limpas seguidas.

## 4. Limites declarados (honestidade)

- `fio_us` chega ao Caderno do cliente nos eventos de leitura sintética
  (híbrido) e fica consultável por `FxpBus::wire_timestamp_of`; a Leiame do
  evento de leitura real ainda não carrega o carimbo — refinamento
  registrado, não v1.1.
- Autenticação PSK não cifra o tráfego (§4.6 do schema declara); rustls é
  o próximo passo do §9.
- Beacon é IPv4 site-local, TTL 1; SSM/IPv6/mDNS fora do escopo (§9).
- O loop de aceitação do peer faz *polling* com granularidade de 5 ms —
  latência de primeira resposta; o throughput do fio (benches acima) não é
  afetado.
