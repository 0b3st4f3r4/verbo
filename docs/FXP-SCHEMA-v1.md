# FXP — Schema de Mensagem v1.1

**Status:** canônico. v1 definido **antes** dos drivers (PLAN §3.5) e canônico
da Etapa 3; **v1.1** (esta revisão, janela `v2027.0.0-alpha.1` — PLAN §8 item 8)
implementa as extensões do fio registradas na antiga §9: compressão, batching
de leituras, descoberta multicast, autenticação do canal remoto e timestamps
absolutos no fio. Implementação de referência: `core/crates/vbl-fxp/src/schema.rs`
(roundtrip em `tests/schema_roundtrip.rs`), máquina de estado de conexão em
`src/transport.rs`, lado servidor em `src/peer.rs`, beacon em `src/discover.rs`.

---

## 1. Princípios

1. **Nomes simbólicos só atravessam o fio.** `cpu_temp`, `CpuPowerCap` etc.
   nunca são substituídos por caminhos de SO na mensagem (FORMAL §3: o
   mapeamento para endpoints é do registro do FXP).
2. **Serialização sem perda.** `encode → decode` é identidade bit a bit para
   todo campo (f64 preserva os 64 bits IEEE-754; strings são UTF-8 exatas).
   `NaN` e `±∞` são **rejeitados no encode** — uma leitura física inválida é
   falha de I/O (FORMAL §4.7), nunca um valor mágico.
3. **Endianness:** little-endian em todos os inteiros e nos bits do f64
   (IEEE-754, ordem LE dos 8 bytes).
4. **Integridade:** a camada FXP **não** adiciona checksum — a integridade do
   transporte é do Unix/TCP (checksum de payload) e a prova de adulteração é
   do Caderno (cadeia SHA-256). Repassar CRC seria custo sem auditoria nova.
5. **Ack por seq.** Toda mensagem pode exigir ack (`FLAG_ACK`); a correlação
   pedido↔resposta é pelo `seq: u32` (wrapping). Timeout sem resposta = falha
   de I/O tratada (nunca valor inventado).
6. **Honestidade de dados no fio** (FORMAL §4.7): a resposta de leitura carrega
   a marca de origem (`FLAG_SINTETICO`); dado sintético só circula em modo
   simulado/híbrido explícito e chega ao Caderno com `measurement_status`.
7. **Recursos novos são negociados e opt-in** (v1.1): o wire de configuração
   default é **bit a bit v1.0** (testado por golden bytes); nenhum frame com
   recurso novo parte sem `CAPS` confirmado; um peer v1.0 diante de opcode
   v1.1 falha **fechado** (`UnknownOpcode`) — nunca interpreta silenciosamente.

## 2. Enquadramento (frame)

```
┌──────────────┬─────────────────────────────┐
│ length: u32  │ payload (length bytes)      │
│ LE, exclui   │ header (12 B) + corpo       │
│ o próprio u32│                             │
└──────────────┴─────────────────────────────┘
```

- `length` ≤ **8192** (guarda anti-inchaço; frame máximo = 8196 bytes no fio).
  Com compressão ativa (§4.8), `length` conta **bytes no fio** (payload já
  comprimido); a região descomprimida também respeita o teto de 8192.
- `length` < 12 ou payload truncado ⇒ **erro de decodificação** (nunca
  mensagem parcial silenciosa).
- Demais violações (magic, versão, opcode desconhecido, campo faltante,
  UTF-8 inválido, `NaN`/`±∞`, algoritmo de compressão desconhecido, bomba de
  descompressão) ⇒ erro de decodificação com razão explícita.

## 3. Header (12 bytes, fixo)

| Offset | Campo        | Tipo       | Valor/Significado                                   |
|-------:|--------------|------------|-----------------------------------------------------|
| 0      | `magic`      | `[u8;3]`   | `"FXP"` (0x46 0x58 0x50)                             |
| 3      | `version`    | `u8`       | `1` — decodificador rejeita versão desconhecida      |
| 4      | `opcode`     | `u8`       | tabela §4                                            |
| 5      | `flags`      | `u8`       | tabela §5                                            |
| 6      | `reservado`  | `u8`       | `0` no encode sem compressão; com `FLAG_COMPRESSED` = id do algoritmo (§4.8). Ignorado no decode quando a flag está ausente. |
| 7      | `name_len`   | `u8`       | 0–255; nome simbólico (0 quando a op não tem nome)   |
| 8      | `seq`        | `u32` LE   | id de correlação ack (wrapping)                      |

Layout do payload após o header (v1.1):

```
header (12 B, sempre plano)
[u64 LE timestamp]   — só quando FLAG_TIMESTAMP (§5): µs desde o epoch UNIX
[região nome+corpo]  — plana, ou comprimida quando FLAG_COMPRESSED (§4.8)
```

O nome viaja **dentro** da região comprimida quando ativa (nomes se repetem
entre frames — é onde a razão de compressão mora); o receiver descomprime
antes de parsear `name_len` + `name` + corpo.

## 4. Opcodes

| Valor | Nome               | Direção        | Corpo após header [+ts] [+decompressão]                 |
|------:|--------------------|----------------|----------------------------------------------------------|
| 0x01  | `READ`             | consumidor→FXP | — (leitura de sensor; alias permitido)                   |
| 0x81  | `READ_OK`          | FXP→consumidor | `f64 LE` (8 B) + `canonical_len u8` + canonical          |
| 0x82  | `READ_ERR`         | FXP→consumidor | `reason u8` (§4.1)                                       |
| 0x02  | `ACT`              | consumidor→FXP | `value_kind u8` + valor (§4.2)                           |
| 0x84  | `ACT_ACK`          | FXP→consumidor | `status u8` + payload por status (§4.3)                  |
| 0x03  | `HEARTBEAT`        | consumidor→FXP | — (sondar ator/dispositivo)                              |
| 0x83  | `HEARTBEAT_ACK`    | FXP→consumidor | `ok u8` (1 = respondeu)                                  |
| 0x04  | `HELLO`            | peer→peer      | registro do peer (§4.4) — handshake/publicação           |
| 0x05  | `BYE`              | peer→peer      | — (encerrar conexão limpa; sem ack)                      |
| 0x06  | `CAPS`             | consumidor→FXP | `u16 LE` capacidades pedidas (§4.5) — v1.1               |
| 0x86  | `CAPS_OK`          | FXP→consumidor | `u16 LE` capacidades concedidas = interseção (§4.5)      |
| 0x07  | `READ_BATCH`       | consumidor→FXP | `u16 count` + count×(`u8 name_len` + name) (§4.7)        |
| 0x87  | `READ_BATCH_OK`    | FXP→consumidor | `u16 count` + count× resultado (§4.7)                    |
| 0x08  | `AUTH_CHALLENGE`   | FXP→consumidor | `u16 LE scheme` + nonce 32 B (§4.6)                      |
| 0x09  | `AUTH_RESPONSE`    | consumidor→FXP | nonce 32 B + HMAC 32 B (§4.6)                            |
| 0x8A  | `AUTH_OK`          | FXP→consumidor | — (handshake aceito; falha = fechamento sem AUTH_OK)     |

### 4.1 `READ_ERR.reason`

| Valor | Razão              | Semântica (FORMAL §4.7)                              |
|------:|--------------------|-------------------------------------------------------|
| 0     | `nao_registrado`   | nome fora do registro — condição não avaliada         |
| 1     | `inacessivel`      | registrado, leitura falhou no endpoint real           |
| 2     | `timeout`          | ack não chegou no prazo                               |
| 3     | `ocupado`          | recurso ocupado (retry permitido)                     |

### 4.2 Valor de comando (`value_kind`)

| Valor | Variante            | Codificação                                   |
|------:|---------------------|-----------------------------------------------|
| 0     | `Num`               | `f64 LE` (8 bytes)                            |
| 1     | `Str`               | `u16 LE len` + bytes UTF-8 (len ≤ 1024)       |
| 2     | `Ident`             | `u16 LE len` + bytes UTF-8 (len ≤ 1024)       |

`Str` e `Ident` preservam a distinção da AST (`"verde"` vs `verde`).

### 4.3 `ACT_ACK.status`

| Valor | Status               | Payload extra                                                |
|------:|----------------------|--------------------------------------------------------------|
| 0     | `Entregue`           | —                                                             |
| 1     | `Rejeitado`          | `limite u8` (0=min, 1=max, 2=safety_limit) + `f64 LE` limite  |
| 2     | `AtorInexistente`    | —                                                             |
| 3     | `Indisponivel`       | — (heartbeat não respondeu)                                  |
| 4     | `FallbackExecutado`  | `u8 len` + nome do ator alternativo                           |
| 5     | `FallbackEsgotado`   | —                                                             |
| 6     | `ValorInvalido`      | `u8 len` + motivo UTF-8 (ex.: cor fora do mapa do LED)        |

Os status espelham 1:1 o `ActOutcome` do `vbl-runtime` (extensão aditiva v1:
`ValorInvalido` — rejeição de valor não numérico fora do domínio do ator).

### 4.4 `HELLO` (publicação de registro)

```
u16 LE  count
count × DeviceDesc:
  kind u8            (0 = sensor, 1 = ator)
  u8 name_len + name
  ── sensor ──
  flags u8           (bit0 tem_min · bit1 tem_max)
  [f64 min] [f64 max]
  u8 g_len + grandeza      u8 u_len + unidade
  f64 precisao_pct         (0.0 = não declarado)
  ── ator ──
  flags u8           (bit0 tem_min · bit1 tem_max · bit2 tem_safety)
  [f64 min] [f64 max] [f64 safety]
```

O `HELLO` transporta o registro do peer remoto para o lado local enxergar os
dispositivos publicados (números usados na validação inclusiva de limites,
FORMAL §4.3).

### 4.5 `CAPS` — negociação de capacidades (v1.1)

```
u16 LE  bitmask de capacidades pedidas
```

| Bit | Capacidade      | Concede ao consumidor                                     |
|----:|-----------------|------------------------------------------------------------|
| 0   | `LZ4`           | frames com `FLAG_COMPRESSED` + algoritmo id 1 (§4.8)       |
| 1   | `BATCH`         | opcodes `READ_BATCH`/`READ_BATCH_OK` (§4.7)                |
| 2   | `TIMESTAMP`     | frames com `FLAG_TIMESTAMP` (§5)                           |

`CAPS_OK` devolve a **interseção** pedidos × suportados. Ordem obrigatória na
conexão: **AUTH (§4.6, se política exigir) → CAPS → HELLO → trabalho**. Nenhum
frame com recurso novo parte sem `CAPS_OK` confirmando a capacidade — o
estado da conexão impõe (cliente e servidor); codec é stateless. Bits 3–15
reservados: `0` no encode; ignorados no decode.

### 4.6 `AUTH_*` — autenticação do canal remoto (v1.1)

```
AUTH_CHALLENGE (servidor→consumidor, enviado ao aceitar a conexão quando
                a política de PSK está ativa):
  u16 LE  scheme     (1 = PSK-HMAC-SHA256; desconhecido ⇒ erro de decode)
  [u8;32] nonce_servidor (aleatório por conexão)

AUTH_RESPONSE (consumidor→servidor):
  [u8;32] nonce_consumidor (aleatório por conexão)
  [u8;32] HMAC-SHA256(chave, "FXP-AUTH1" ‖ nonce_consumidor ‖ nonce_servidor)

AUTH_OK (servidor→consumidor): corpo vazio, mesmo seq da RESPONSE.
```

- Chave pré-compartilhada (PSK) de **env** (`psk:NOME_DA_VAR`) — nunca no
  arquivo de config. Verificação em tempo constante (`hmac::verify`).
- Política fail-closed: com PSK ativa, o servidor **não processa** nenhuma
  mensagem antes do `AUTH_OK` (qualquer outra opcode ⇒ fechamento da conexão).
  Chave errada ⇒ fechamento limpo sem `AUTH_OK` (sem razão no fio — não
  alimentar sondagem).
- **Escopo honesto:** isto autentica o par na abertura da conexão. **Não** cifra
  e **não** autentica frames individuais — confidencialidade e MAC por frame
  (rustls) ficam registrados como trabalho futuro (§9). A integridade do
  fluxo segue sendo a do Unix/TCP (princípio 4).
- Replay: nonces frescos por conexão tornam a MAC não reutilizável.

### 4.7 `READ_BATCH` — batching de leituras (v1.1)

```
READ_BATCH:  u16 LE count (1..=64)  + count × (u8 name_len + name)
READ_BATCH_OK: u16 LE count (igual ao pedido)
             + count × resultado:
                 u8 status       (0 = ok; 1..=3 = razões de §4.1;
                                  4 = nao_registrado — o byte 0 do item é o
                                  status "ok", logo a razão 0 de §4.1 viaja
                                  como tag 4 para não colidir)
                 ok   → f64 LE valor + u8 canonical_len + canonical
                 erro → (nada)
```

- Um único `seq` (com `FLAG_ACK`) correlaciona o lote inteiro.
- `count` > 64 ⇒ erro de decodificação; frame respeita o teto de §2.
- Item com erro **nunca** vira valor (§4.1): status espelha `READ_ERR.reason`.
- Uso previsto: prefetch do barramento no primeiro cache-miss do tick
  (`batch_prefetch`, default **off**). Semântica do Caderno preservada: o
  evento semântico de leitura é do runtime; o lote gera só o evento
  diagnóstico `fxp_batch`. Falha pré-buscada de sensor que o programa não
  pediu **não** gera alerta — o alerta continua pertencendo à pergunta feita.

### 4.8 Compressão do corpo (v1.1)

- Algoritmo único no v1.1: **LZ4 block** (`id 1`, byte `reservado` do header).
  Threshold de encode: só comprime quando a região plana (ts+nome+corpo)
  excede **512 B** e o resultado não excede a região plana (nunca inflar o fio).
- `FLAG_COMPRESSED` marca o frame; `CAPS` bit 0 autoriza. Sem negociação, o
  estado da conexão **proíbe o envio** (nunca depende de o outro lado
  "tolerar" flag desconhecida).
- Guarda de bomba: a região descomprimida ≤ 8192; excedeu ou blob corrupto ⇒
  erro de decodificação (`DecompressionFailed`), nunca execução parcial.

### 4.9 Beacon `FXPD` — descoberta multicast (v1.1)

Datagrama UDP único (sem length-prefix, sem ack — UDP é lossy; liveness fica
no heartbeat/TCP), anunciando a existência de um servidor FXP:

```
magic [u8;4] = "FXPD"
u8      versão        (1)
u16 LE  porta TCP
u8      id_len + identifier UTF-8 (≤ 255; nome do servidor)
u32 LE  primeiros 4 bytes de SHA-256 dos nomes canônicos do registro,
        ordenados e concatenados com \n (impressão digital do registro)
```

- Grupo `239.255.70.80:7080` (IPv4, escopo site-local), TTL 1, intervalo 2 s.
- O anúncio **não carrega dado de sensor**; o canal segue o fluxo
  AUTH→CAPS→HELLO do §4.5/§4.6 sobre TCP.
- Opt-in (anúncio e consumo) via config; default **off** para não introduzir
  dependência de rede nos testes determinísticos de CI. Rede sem multicast ⇒
  descoberta "indisponível" (caminho honesto §4.7 — nunca erro de construção).

## 5. Flags (u8)

| Bit | Nome            | Significado                                                |
|----:|-----------------|------------------------------------------------------------|
| 0   | `FLAG_ACK`      | remetente exige resposta com o mesmo `seq`                 |
| 1   | `FLAG_ERRO`     | resposta de erro (`READ_ERR`, ack negativo)                |
| 2   | `FLAG_FALLBACK` | entrega efetivada por ator alternativo (FORMAL §4.3)       |
| 3   | `FLAG_SINTETICO`| dado de origem simulada (`measurement_status`) — §4.7      |
| 4   | reservado       | `0` no encode; ignorado no decode (futuro)                 |
| 5   | `FLAG_TIMESTAMP`| payload carrega `u64 LE` µs desde o epoch UNIX (§3) — v1.1 |
| 6   | `FLAG_COMPRESSED`| região nome+corpo comprimida em LZ4 block (§4.8) — v1.1   |
| 7   | reservado       | `0` no encode; ignorado no decode                          |

`FLAG_TIMESTAMP` é **anotação de laboratório**: o Caderno permanece no relógio
virtual do runtime (`tick`/`t` reservados — NOTEBOOK-FORMAT §3). O peer carimba
o instante físico da leitura/escrita do driver; o consumidor propaga o valor
como metadado (`fio_us`) e correlaciona com medição externa (RAPL/wattímetro,
Etapa 5). Sem sincronização de relógio entre hosts o valor não é verdade
causal — ausência de `FLAG_TIMESTAMP` (peer antigo) é honesta e esperada.

## 6. Timeouts e políticas (defaults da Etapa 3 + v1.1)

| Parâmetro               | Default     | Orçamento (AGENTS §1.2 EIF)                     |
|-------------------------|-------------|--------------------------------------------------|
| `read_timeout` (fio)    | 10 ms       | leitura local ≤ **1 ms** p95; remota ≤ **10 ms** p95 |
| `act_timeout` local     | 50 ms       | ack local ≤ **50 ms** p95                        |
| `act_timeout` remoto    | 500 ms      | ack remoto ≤ **500 ms** p95                      |
| `cache_ttl`             | 100 ms      | cache de leitura real (PLAN §3, mitigação)       |
| `retries` (transporte)  | 1           | PLAN §3: fila com retry e fallback               |
| `queue_timeout`         | 2 ticks     | comando pendente expira com evento no Caderno    |
| `batch_prefetch` (v1.1) | off         | 1 RTT por lote ≤ 64 sensores, dentro do ≤ 10 ms  |
| `compress_threshold` (v1.1) | 512 B   | só payload maior; nunca inflar o fio             |

Timeouts de leitura/ack medem **tempo de parede** e se aplicam ao transporte
com fio (Unix/TCP); a fronteira in-process é chamada direta (orçamento
≤ 10 µs/mensagem). A fila de retry avança no **relógio virtual** (`on_tick`)
para manter os testes determinísticos em CI.

Ordem de abertura de conexão (v1.1): `AUTH? → CAPS → HELLO → trabalho`.

Política de entrega de `act` no bus (inalterada):

1. **Validação local** de limites (inclusiva) — rejeição sem envio;
2. entrega na rota primária (1 retry de transporte);
3. indisponível ⇒ **fallback do registro** (primary → alternativos;
   FORMAL §4.3 — o runtime não implementa fallback próprio);
4. fallback esgotado ⇒ comando entra na **fila prioritária** para retry em
   ticks futuros até `queue_timeout` (evento no Caderno ao expirar).

Prioridade da fila: `0` = máxima (comandos associados a `subvert` — FORMAL
§4.5: atuação pós-subvert sem atraso perceptível), `10` = normal.

## 7. Transportes

| Transporte | Enquadramento | Uso                                          |
|------------|---------------|----------------------------------------------|
| `in-process` | chamada direta (sem fio) | modo local; orçamento ≤ 10 µs/mensagem |
| `unix`     | frames §2 sobre `UnixStream` (SOCK_STREAM) | local entre processos (`fxpd`) |
| `tcp`      | frames §2 sobre `TcpStream` | remoto — mesma semântica, timeouts maiores |
| `udp-beacon` (v1.1) | datagrama FXPD §4.9 (sem ack) | descoberta multicast, anúncio only |

O servidor de referência (testes/integração) fala exatamente o frame §2;
nenhuma mensagem v1 é transport-specific. O lado servidor com estado de
protocolo (AUTH/CAPS/compressão/batch) é o `PeerServer` (`src/peer.rs`),
dono canônico único da máquina de estados — os loops genéricos
`serve_unix`/`serve_tcp` continuam canos sem semântica de protocolo.

## 8. Matriz opcode × mensagem obrigatória (rastreabilidade)

| Requisito (PLAN/FORMAL/§9) | Mensagem v1.1                      | Teste |
|-----------------------------|------------------------------------|-------|
| serializa/desserializa sem perda (AGENTS Etapa 3) | todas | `tests/schema_roundtrip.rs` |
| falha de sensor nunca é 0.0 (§4.7) | `READ_ERR` | `tests/schema_roundtrip.rs` |
| dado sintético marcado (§4.7) | `FLAG_SINTETICO` | `tests/schema_roundtrip.rs` |
| limites inclusivos (§4.3) | `ACT_ACK.Rejeitado` | `tests/bus.rs` |
| fallback do registro (§4.3) | `FLAG_FALLBACK` + `FallbackExecutado` | `tests/bus.rs` |
| heartbeat (BDD Caso 3) | `HEARTBEAT`/`HEARTBEAT_ACK` | `tests/transport.rs` |
| aliases com nome canônico (§6) | `READ_OK.canonical` | `tests/registry.rs` |
| prioridade pós-subvert (§4.5) | fila prioritária | `tests/queue.rs` |
| negociação antes de recurso (§4.5) | `CAPS`/`CAPS_OK` | `tests/transport.rs` |
| peer v1.0 falha fechado (princípio 7) | opcodes novos × decoder v1 | `tests/transport.rs` |
| timestamp físico é anotação (§5) | `FLAG_TIMESTAMP` → `fio_us` | `tests/bus.rs` |
| lote ≤ 64, erro por item honesto (§4.7) | `READ_BATCH(_OK)` | `tests/schema_roundtrip.rs` + `tests/bus.rs` |
| compressão só negociada, sem bomba (§4.8) | `FLAG_COMPRESSED` | `tests/schema_roundtrip.rs` + `tests/transport.rs` |
| PSK fail-closed (§4.6) | `AUTH_*` | `tests/transport.rs` |
| anúncio sem dado de sensor (§4.9) | beacon `FXPD` | `tests/discover.rs` |

## 9. Extensões (estado da v1.1)

**Implementado na v1.1** (antes listadas como trabalho futuro na antiga §9):
compressão (§4.8), batching de leituras (§4.7), descoberta multicast (§4.9),
autenticação do canal remoto (§4.6) e timestamps absolutos no fio (§3/§5 —
timestamp físico é atribuição de laboratório; o Caderno usa o relógio virtual
do runtime).

**Fica registrado como trabalho futuro (fora da v1.1):** confidencialidade e
MAC por frame via TLS (rustls), IPv6/SSM para o beacon, mDNS/DNS-SD,
dicionários de compressão compartilhados entre frames.
