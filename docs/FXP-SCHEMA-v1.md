# FXP — Schema de Mensagem v1.4

**Status:** canônico. v1 definido **antes** dos drivers (PLAN §3.5) e canônico
da Etapa 3; **v1.1** (janela `v2027.0.0-alpha.1` — PLAN §8 item 8) implementou
as extensões do fio da antiga §9: compressão, batching de leituras, descoberta
multicast, autenticação do canal remoto e timestamps absolutos no fio.
**v1.2** (PLAN §8 item 9) implementou as quatro extensões registradas na §9 da
v1.1: **TLS 1.3 com pinning** (§7, `tcps`), **dicionário de compressão
compartilhado** (§4.8), **beacon IPv6 + SSM IPv4** (§4.9) e **mDNS/DNS-SD**
(§4.10). **v1.3** (PLAN §8 item 10) implementou as quatro extensões que
ficaram registradas na §9 da v1.2: **SSM IPv6** para o beacon (§4.9,
`MCAST_JOIN_SOURCE_GROUP` da RFC 3678), **TOFU** como alternativa
operacional ao pinning (§7), **zstd com dicionário treinado** (§4.8, id 3,
bit `ZSTD`) e **resumo de sessão + 0-RTT TLS** (§7 — o frame `CAPS` pode
partir junto do `ClientHello`). **v1.4** (esta revisão — PLAN §8 item 11)
implementa as cinco extensões registradas na §9 da v1.3: **TOFU estrito**
(`accept-new`, §7 — allow-list que nunca aprende), **rotação de pins com
sobreposição** (§7 — multi-pin `@sha256:H1,H2`), **verificação de
dicionário no fio** (`DICT_SYNC`, §4.8 — id 4, bit `ZSTD_V`; pontas com
versões de zstd diferentes negociam compatibilidade em vez de quebrar),
**sessão retomada entre processos** (§7 — cache de tickets do SERVIDOR em
disco, `--tls-sessions`) e o **benchmark de 0-RTT com RTT real** (§9 —
proxy que injeta atraso; números no relatório v1.4). O fio default (sem
recursos negociados) permanece **bit a bit v1.0**; todo recurso novo segue
o princípio 7 (negociado, opt-in, fail-closed). Implementação de referência:
`core/crates/vbl-fxp/src/schema.rs` (roundtrip em `tests/schema_roundtrip.rs`),
máquina de estado de conexão em `src/transport.rs`, lado servidor em
`src/peer.rs`, beacon em `src/discover.rs`, mDNS em `src/mdns.rs`
(feature `mdns`), TLS/TOFU/sessões em `src/tls.rs` e `src/sessoes.rs`;
cenários v1.4 em `tests/v14.rs`.

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
7. **Recursos novos são negociados e opt-in** (v1.1; mantido na v1.2): o wire
   de configuração default é **bit a bit v1.0** (testado por golden bytes);
   nenhum frame com recurso novo parte sem `CAPS` confirmado; um peer v1.0
   diante de opcode v1.1 falha **fechado** (`UnknownOpcode`) — nunca
   interpreta silenciosamente. As promoções de bits de `CAPS` (bit 3
   reservado na v1.1 ⇒ `DICT` na v1.2; bit 4 reservado na v1.2 ⇒ `ZSTD` na
   v1.3) são seguras **por construção**: decoders antigos ignoram bits
   reservados no decode; quem não negocia `DICT` vê o id 2 como desconhecido
   e quem não negocia `ZSTD` vê o id 3 como desconhecido (§4.8) — todos
   falham fechado, nunca interpretam silenciosamente.

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
| 0x0A  | `DICT_SYNC`        | consumidor→FXP | `zstd_version u32 LE` + `dict_hash 32 B` (§4.8) — v1.4   |
| 0x8B  | `DICT_SYNC_OK`     | FXP→consumidor | `zstd_version u32 LE` + `dict_hash 32 B` (§4.8) — v1.4   |

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
FORMAL §4.3). **Na v1.2** o `HELLO` ganha um segundo papel: quando `DICT`
(§4.5) foi concedido, o `HELLO` integra o handshake (o §4.5 já o ordenava) e
ambos os lados derivam o **mesmo** dicionário do registro do SERVIDOR a
partir do que cada lado já possui — servidor: o próprio registro; cliente: a
resposta `HELLO`. Nenhum byte de dicionário cruza o fio (§4.8).

### 4.5 `CAPS` — negociação de capacidades (v1.1)

```
u16 LE  bitmask de capacidades pedidas
```

| Bit | Capacidade      | Concede ao consumidor                                     |
|----:|-----------------|------------------------------------------------------------|
| 0   | `LZ4`           | frames com `FLAG_COMPRESSED` + algoritmo id 1 (§4.8)       |
| 1   | `BATCH`         | opcodes `READ_BATCH`/`READ_BATCH_OK` (§4.7)                |
| 2   | `TIMESTAMP`     | frames com `FLAG_TIMESTAMP` (§5)                           |
| 3   | `DICT` (v1.2)   | algoritmo id 2 com dicionário do registro (§4.8); o `HELLO` passa a integrar o handshake |
| 4   | `ZSTD` (v1.3)   | algoritmo id 3 com dicionário **treinado** (§4.8); negociado sempre JUNTO com `DICT` — o gatilho do `HELLO` é o mesmo |
| 5   | `ZSTD_V` (v1.4) | algoritmo id 4 com dicionário treinado **verificado no fio** (§4.8 — `DICT_SYNC`); negociado sempre JUNTO com `ZSTD + DICT` |

`CAPS_OK` devolve a **interseção** pedidos × suportados. Ordem obrigatória na
conexão: **AUTH (§4.6, se política exigir) → CAPS → HELLO → trabalho** — com
0-RTT TLS (§7) o `CAPS` pode partir JUNTO do `ClientHello`, mas a ordem
lógica é a mesma e a resposta `CAPS_OK` continua obrigatória. Nenhum frame
com recurso novo parte sem `CAPS_OK` confirmando a capacidade — o estado da
conexão impõe (cliente e servidor); codec é stateless. Com `ZSTD_V`
concedido, entra na ordem, após o `HELLO`, o `DICT_SYNC` (§4.8) — o id 4 só
trafega depois do hash casado. Bits 6–15 reservados: `0` no encode; ignorados
no decode (o bit 3 era reservado na v1.1, o bit 4 na v1.2 e o bit 5 na
v1.3 — as promoções são compatíveis, ver princípio 7).

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

### 4.8 Compressão do corpo (v1.1; id 2 v1.2; id 3 v1.3; id 4 v1.4)

- Algoritmos no byte `reservado` do header: `id 1` = **LZ4 block** (v1.1);
  `id 2` = **LZ4 block + dicionário compartilhado do registro** (v1.2);
  `id 3` = **zstd + dicionário TREINADO do registro** (v1.3);
  `id 4` = **zstd + dicionário treinado VERIFICADO no fio** (v1.4).
  Threshold de encode: só comprime quando a região plana (ts+nome+corpo)
  excede **512 B** e o resultado não excede a região plana (nunca inflar o fio).
- `FLAG_COMPRESSED` marca o frame; `CAPS` bit 0 autoriza o id 1; bit `DICT`
  (§4.5) autoriza o id 2; bits `DICT + ZSTD` autorizam o id 3; bits
  `DICT + ZSTD + ZSTD_V` autorizam o id 4 — mas o id 4 SÓ trafega depois do
  `DICT_SYNC` com hash casado (abaixo). Sem negociação, o estado da conexão
  **proíbe o envio** (nunca depende de o outro lado "tolerar" flag
  desconhecida). Com id 3 concedido, o encoder prefere o id 3 ao id 2
  (razão maior com o mesmo gatilho `HELLO`); com id 4 liberado, prefere o
  id 4; a degradação inversa (peer v1.3 sem `ZSTD_V`) cai no id 3 pela
  interseção de `CAPS_OK`.
- **Dicionário (v1.2):** derivado deterministicamente do registro do
  SERVIDOR — nomes canônicos **ordenados**, concatenados com `\n`, teto
  **64 KiB** (truncado em ordem; mesmos bytes dos dois lados). O servidor
  usa o próprio registro; o cliente deriva da resposta `HELLO`. Regras de
  estado: (a) a resposta `HELLO` nunca sai comprimida com dict (o cliente
  só terá o dicionário depois de recebê-la); (b) o lado servidor só marca o
  dicionário como pronto DEPOIS de receber o `HELLO` do cliente; (c) frames
  de trabalho com id 2 só partem após o gatilho do `HELLO`, dos dois lados.
- **Dicionário treinado (v1.3, id 3):** a MESMA matéria (nomes canônicos
  ordenados do registro do servidor) vira **amostras** do treino COVER
  (`zstd::dict::from_samples`), teto **16 KiB** — zstd extrai estatística,
  não matéria crua, e 16 KiB já cobre o frame-teto (8 KiB) com folga.
  Nível do fio fixo: **3** (constante da especificação). O treino é
  determinístico para (nomes, versão do zstd): pontas com versões de zstd
  diferentes podem derivar dicionários diferentes — **divergência ⇒
  `DecompressionFailed`** (fail closed, honesto; nunca lixo silencioso).
  Registro pequeno demais para o COVER treinar (poucos bytes de amostra) ⇒
  o servidor **não concede** `ZSTD` (o bit sai da interseção — degradação
  explícita, nunca dicionário vazio no fio); a derivação acontece na fase de
  handshake, que tem prazo próprio (§6). Medição no payload canônico do
  bench (lote de 40 leituras canônicas, 2014 B planos): id 2 = 361 B no
  fio (5,6×), id 3 = 298 B (6,8×); dicionários derivados: concatenação
  1639 B × treinado 1321 B. Custo do treino: ~5 ms **uma vez por
  derivação** (handshake), não por frame (bench `zstd_treino_dict_41_nomes`).
- **Verificação de dicionário no fio (v1.4, id 4 + `DICT_SYNC`):** o §9 da
  v1.3 registrava o problema real — o treino COVER é determinístico por
  (nomes, VERSÃO DO ZSTD), então pontas com libzstd diferentes derivam
  dicionários diferentes e o id 3 quebra com `DecompressionFailed` DEPOIS
  do handshake. A v1.4 torna a compatibilidade NEGOCIÁVEL: concedido o bit
  `ZSTD_V` (§4.5), após o `HELLO` o cliente envia `DICT_SYNC
  {zstd_version: u32 LE, dict_hash: 32 B}` — a versão da sua libzstd
  (`zstd_safe::version_number()`) e o `SHA-256` do dict treinado que
  derivou. O servidor responde `DICT_SYNC_OK` com O SEU par
  (versão, hash). Hashes iguais ⇒ o id 4 fica habilitado NOS DOIS SENTIDOS
  (as respostas partem com id 4; frames id 4 do cliente decodificam).
  Hashes diferentes (ou versões de zstd diferentes) ⇒ o cliente se mantém
  no id 3 **só se o peer o concedeu**; peer v1.3 (sem `ZSTD_V`) nem
  chega ao `DICT_SYNC` — conexão fica no id 3 normal. Degradar do id 4 ⇒
  evento honesto `fxp_dict_divergente` no Caderno com as duas versões.
  `DICT_SYNC` sem `ZSTD_V` concedido é violação de estado (ignorado —
  mesmo tratamento de recurso não negociado, §4.5). Fail-closed por
  construção do TIPO: o codec só decodifica id 4 diante do dicionário
  treinado VERIFICADO (`DictConexao::ZstdV`) — id 4 com a matéria dos ids
  2/3 ⇒ `UnknownCompression { received: 4 }`; divergência pós-liberação ⇒
  `DecompressionFailed`. O treino em si é idêntico ao do id 3 (COVER,
  teto 16 KiB, nível 3).
- Fail-closed: decoder sem dicionário diante do id 2 ⇒ `UnknownCompression
  { received: 2 }` — **idêntico ao comportamento v1.1** (o codec é stateless
  e desconhece o id); diante do id 3 sem dicionário treinado (ou com a
  matéria do id 2) ⇒ `UnknownCompression { received: 3 }` — **idêntico ao
  comportamento v1.2** (o tipo do dicionário na conexão casa com o algoritmo:
  id 2 exige a matéria concatenada, id 3 exige a treinada); diante do id 4
  sem o dicionário treinado VERIFICADO (ou com matéria dos ids 2/3) ⇒
  `UnknownCompression { received: 4 }` — **idêntico ao comportamento v1.3**;
  dicionário divergente ou blob corrupto ⇒ `DecompressionFailed`. Nunca lixo
  silencioso.
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

- Grupos: `239.255.70.80:7080` (IPv4, escopo site-local) e
  `[ff35::7080]:porta` (IPv6, escopo site-local 0x35) — TTL 1, intervalo 2 s.
- **SSM (assinatura por fonte, RFC 4607/3678):** o consumidor assina a FONTE
  em vez de receber de todos: grupo IPv4 `ip:porta@fonte-v4` (v1.2) e grupo
  IPv6 `[v6]:porta@[fonte-v6%N]` (v1.3 — join com `MCAST_JOIN_SOURCE_GROUP`
  da RFC 3678, Linux; em SO sem a opção o erro é honesto:
  "multicast indisponível neste host"). Fonte é sempre da MESMA família do
  grupo (fonte v6 em grupo v4 ⇒ erro de parse honesto). Sintaxe no config:
  `[ff35::7080]:porta@[fe80::1%2]` — scope de LINK-LOCAL é numérico
  (`%N`), nunca o nome da interface (o arquivo de config não é `/proc`).
- O anúncio **não carrega dado de sensor**; o canal segue o fluxo
  AUTH→CAPS→HELLO do §4.5/§4.6 sobre TCP.
- Opt-in (anúncio e consumo) via config; default **off** para não introduzir
  dependência de rede nos testes determinísticos de CI. Rede sem multicast ⇒
  descoberta "indisponível" (caminho honesto §4.7 — nunca erro de construção).

### 4.10 mDNS/DNS-SD (v1.2, opt-in por feature)

Alternativa ao beacon UDP (§4.9) para redes que filtram multicast IP mas
resolvem mDNS. Compilada **apenas** com a cargo feature `mdns` (default off —
nenhuma dependência nova no build de produção sem opt-in):

- Serviço **`_fxp._tcp.local.`**, instância = identifier do `fxpd`; TXT:
  `id` (identificador, canônico para match), `hash` (hex dos primeiros 4
  bytes do SHA-256 do registro — mesma impressão digital do beacon §4.9) e,
  para peers TLS (§7), `tls=1` + `pin` (hex SHA-256 do DER do certificado).
- O endpoint `mdns:<identificador>` no registro resolve no `build()` do
  barramento (janela idêntica ao beacon); TXT `tls=1` ⇒ endpoint `tcps`
  com o pin. Sem a feature, o parse de `mdns:` **rejeita** o endpoint com
  erro honesto — nada de aceitar e falhar depois.
- mDNS é lossy como UDP (§4.9): ausência de resposta não é recusa; sem
  cache além da janela de escuta.

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
| `handshake` (v1.3)      | max(timeout da conexão, **500 ms**) | AUTH+CAPS+HELLO incluem a DERIVAÇÃO do dicionário (treino COVER ~5 ms, §4.8) e o handshake TLS — prazo próprio, não o de leitura |
| `tls_handshake` (v1.2)  | 2 s         | handshake TLS puro com verificação de pin/TOFU   |

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
| `tcps` (v1.2) | frames §2 sobre **TLS 1.3** (rustls, provedor `ring`) | remoto com confidencialidade + MAC; **nunca degrada** para texto plano; v1.3: TOFU, resumo de sessão e 0-RTT (§7); v1.4: TOFU estrito, multi-pin com sobreposição e sessões do servidor em disco (§7) |
| `udp-beacon` (v1.1) | datagrama FXPD §4.9 (sem ack) | descoberta multicast, anúncio only; v1.2: grupos IPv6 e SSM IPv4 (§4.9); v1.3: SSM IPv6 (§4.9) |
| `mdns` (v1.2, feature) | DNS-SD §4.10 | descoberta alternativa ao beacon |

**TLS (`tcps`, v1.2; TOFU/0-RTT v1.3):** o FXP não tem PSK no TLS (rustls
não expõe TLS-PSK — rustls/rustls#174); o modelo de confiança v1.2 é
**certificado autoassinado + pinning por impressão digital**: o endpoint
`tcps:host:porta@sha256:HEX` carrega o pin SHA-256 do DER do certificado e o
cliente REJEITA o handshake cujo certificado não bate exatamente com o pin
(fail-closed; sem fallback para texto plano). **v1.3 — TOFU (trust on first
use)** como alternativa OPERACIONAL ao pinning: o endpoint
`tcps:host:porta@tofu` não copia pin nenhum para o config; a impressão
digital vista na PRIMEIRA conexão é gravada num store local
(`--tofu-store ARQUIVO` no CLI; default
`$XDG_STATE_HOME`/`~/.local/state` + `/verbo/fxp-known-hosts.json`, JSON
determinístico `{"host:porta":"sha256:hex"}`, escrito atomicamente via
rename) e as conexões seguintes verificam contra ela. Divergência (outro
certificado no mesmo host:porta) ⇒ falha fechada com motivo TOFU no Caderno
— a semântica de segurança é a do `known_hosts` do SSH. Store ausente ou
corrupto ⇒ falha fechada da conexão (nunca confiar sem poder registrar).
Unix + TLS é recusado honestamente no arranque (não faz sentido empilhar
camadas).

**v1.4 — TOFU estrito (`accept-new`) e rotação de pins (§7):**

1. **TOFU estrito** (`tcps:host:porta@tofu-estrito`): o modo v1.3 aprende a
   primeira impressão digital; o estrito NUNCA aprende — o alvo precisa já
   existir no store (allow-list operacional: o dono do endpoint registra o
   pin ANTES da primeira conexão, exatamente o `accept-new` recusado do SSH).
   Alvo ausente no store ⇒ `TofuFalha::Desconhecida` e conexão recusada;
   presente ⇒ qualquer pin registrado da entrada vale (o store aceita as
   três formas de entrada: legada `"host:porta":"sha256:hex"`, nova
   `{"pins":["sha256:h1","sha256:h2"]}` e a mistura — carga honesta das
   duas gerações).
2. **Rotação de certificado com sobreposição** (`tcps:host:porta@sha256:H1,H2`):
   a lista de pins existe para a JANELA de rotação — o operador adiciona o
   pin do certificado NOVO mantendo o VELHO, troca o certificado no
   servidor (clientes com pin duplo continuam conectando DURANTE a troca) e
   remove o pin velho DEPOIS (API `adicionar_pin`/`remover_pin` no store;
   teto de 8 pins por endpoint — sobreposição de rotação, não lista de
   confiança). Reparse: `description()` do endpoint devolve
   `tcps:host:porta@sha256:H1,H2` (roundtrip de config).
3. **Formato do store:** entrada com um pin só continua sendo escrita no
   formato legado `{"host:porta":"sha256:hex"}` (bit a bit compatível com a
   v1.3); dois ou mais pins ⇒ `{"host:porta":{"pins":["sha256:h1",…]}}`.
   Parser aceita as duas formas e a mistura.

**Sessão retomada entre PROCESSOS (v1.4 §7):** o §9 da v1.3 registrava que
o cache de tickets era só em memória — o `fxpd` que renasce (deploy, crash,
restart) perdia as sessões e todo cliente pagava handshake completo. A
v1.4 persiste o storage de sessões do SERVIDOR em disco
(`fxpd --tls-sessions ARQUIVO`): o rustls 0.23 trafega o estado de sessão
do servidor como bytes crus (trait `StoresServerSessions`), então o cache
(`src/sessoes.rs`, `CacheSessoesDisco`) grava cada `put`/`take`
atomicamente (`.tmp` + rename, permissão `0600` — blob de sessão é material
de retomada: quem lê o arquivo pode retomar), com evicção do mais velho
acima do teto (1024, a mesma ordem do cache em memória do rustls) e poda
por idade (7 dias, teto do TLS para tickets). O cliente que renasce NÃO
retoma (ticket do cliente em disco não é possível no rustls 0.23 —
`Tls13ClientSessionValue` é opaco, rustls/rustls#2287; a API que falta só
entra na 0.24 — PR rustls#2907; registrado como trabalho futuro na §9).
Store de sessões corrompido no arranque ⇒ falha honesta do `fxpd` (nunca
recomeçar silencioso material de sessão). O 0-RTT da v1.3 segue intacto:
storage stateful é justamente o que o rustls EXIGE para early data (um
ticketer stateless desligaria o 0-RTT — server/tls13.rs) — persistir em
disco mantém os DOIS ganhos: retomada entre processos E 0-RTT.

**Resumo de sessão e 0-RTT (v1.3):** o `ClientConfig` do cliente é cacheado
por chave de confiança (pin ou `host:porta@tofu`) — com o cache em memória
do rustls (`Resumption::in_memory_sessions`, 256 sessões) a SEGUNDA conexão
ao mesmo peer retoma a sessão (`handshake_kind() = Resumed`), eliminando a
troca de certificado. Com **0-RTT** (`enable_early_data`, teto
`EARLY_DATA_MAX = 512 B` de aplicação), o frame `CAPS` do cliente pode partir
JUNTO do `ClientHello`: se o servidor aceitar os dados adiantados, o
`CAPS_OK` chega no primeiro voo e a negociação custa zero RTTs extras; se
recusar (sessão nova, servidor sem 0-RTT), o frame simplesmente não foi
entregue e o cliente **renegocia normalmente** — degradação honesta, nunca
deadlock. Honestidade sobre replay: 0-RTT TLS é replayável por um atacante
de rede; o único frame que viaja adiantado é o `CAPS` **idempotente por
conexão** (a conexão só existe se o handshake completar; `CAPS` duplicado é
absorvido pela máquina de estados) e NUNCA `ACT` nem `READ` — atuação e
leitura seguem exigindo handshake completo (1-RTT). Com AUTH+PSK ativo, o
cliente não adianta `CAPS`: o servidor fala primeiro (`AUTH_CHALLENGE`,
§4.6), então a ordem do §4.5 é preservada. O handshake tem timeout próprio
(§6).

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
| TLS pin certo conecta; errado falha fechado (§7, v1.2) | `tcps` + pin | `tests/v12.rs` + `tests/e2e.rs` (CLI) |
| TLS nunca em Unix; plano↔TLS cross falha nas duas direções (§7) | recusa honesta | `tests/v12.rs` |
| dict derivado do registro, roundtrip id 2 (§4.8, v1.2) | `FLAG_COMPRESSED` + id 2 | `tests/v12.rs` + `tests/schema_roundtrip.rs` |
| id 2 sem dict falha como v1.1 (§4.8, v1.2) | `UnknownCompression{2}` | `tests/v12.rs` |
| dict negociado com HELLO; degradação v1.1 (§4.8) | `CAPS` bit 3 | `tests/v12.rs` |
| beacon IPv6 e SSM IPv4; parse honesto (§4.9, v1.2) | grupos alternativos | `tests/v12.rs` |
| mDNS anúncio/resolve; endpoint `mdns:` exige feature (§4.10) | TXT id/hash/tls/pin | `tests/v12.rs` (feature `mdns`) |
| SSM IPv6 com fonte escopada; família trocada falha no parse (§4.9, v1.3) | `[v6]:porta@[fonte%N]` | `tests/v13.rs` |
| sessão TLS retoma (Full→Resumed) com o mesmo pin (§7, v1.3) | `handshake_kind` | `tests/v13.rs` |
| CAPS como 0-RTT aceito; sem 0-RTT renegocia honesta (§7, v1.3) | `CAPS` adiantado | `tests/v13.rs` |
| TOFU: 1ª use grava, 2ª verifica, divergência falha fechado no Caderno (§7, v1.3) | `@tofu` + store | `tests/v13.rs` + `tests/e2e.rs` (CLI) |
| store TOFU corrupto falha abertura; hex puro e `sha256:` carregam (§7, v1.3) | JSON determinístico | `tests/v13.rs` |
| dict treinado determinístico; id 3 roundtrip; threshold e nunca inflar (§4.8, v1.3) | id 3 | `tests/v13.rs` |
| id 3 sem dict treinado falha como v1.2; id 2 com dict treinado falha (§4.8, v1.3) | `UnknownCompression{3}/{2}` | `tests/v13.rs` |
| zstd negociado com DICT; sem treino/bit sai da interseção; degrada no id 2 (§4.8, v1.3) | `CAPS` bits 3+4 | `tests/v13.rs` |
| id 4 roundtrip fail-closed tipado; bit 5 vira ZSTD_V, reservados 6–15 (§4.5/§4.8, v1.4) | id 4 / `caps::ZSTD_V` | `tests/v14.rs` + `tests/schema_roundtrip.rs` |
| `DICT_SYNC` no fio; hash casado libera id 4 nos dois sentidos (§4.8, v1.4) | `DICT_SYNC`/`DICT_SYNC_OK` | `tests/v14.rs` |
| hash divergente ⇒ degrada honesta para id 3, sem frame id 4 (§4.8, v1.4) | `DICT_SYNC_OK` divergente | `tests/v14.rs` |
| peer v1.3 não concede bit 5; caminho id 3 permanece intacto (§4.5, v1.4) | interseção `CAPS_OK` | `tests/v14.rs` |
| multi-pin parse/reparse; pins malformados recusados (§7, v1.4) | `@sha256:H1,H2` | `tests/v14.rs` |
| TOFU estrito: endpoint e store (legado/novo/misto); sem entrada falha com motivo (§7, v1.4) | `@tofu-estrito` | `tests/v14.rs` |
| aprendizagem v1.3 intacta com múltiplos pins no store (§7, v1.4) | store multi-pin | `tests/v14.rs` |
| rotação de certificado com sobreposição de pins, e2e (§7, v1.4) | pins duplos | `tests/v14.rs` |
| e2e estrito: sem entrada falha fechado; semeado conecta (§7, v1.4) | bus + `@tofu-estrito` | `tests/v14.rs` |
| sessão retoma entre RENASCIMENTOS do processo; 0-RTT aceito (§7, v1.4) | `--tls-sessions` | `tests/v14.rs` |
| cache de sessões em disco: put/take/persistência/evicção; corrupto falha (§7, v1.4) | `CacheSessoesDisco` | `tests/v14.rs` |
| 0-RTT quantificado com RTT real > 1 ms (§9, v1.4) | proxy de atraso | `benches/fxp.rs` (`v14_tls_0rtt_rtt`) |

## 9. Extensões (estado da v1.4)

**Implementado na v1.1** (antes listadas como trabalho futuro na antiga §9):
compressão (§4.8), batching de leituras (§4.7), descoberta multicast (§4.9),
autenticação do canal remoto (§4.6) e timestamps absolutos no fio (§3/§5 —
timestamp físico é atribuição de laboratório; o Caderno usa o relógio virtual
do runtime).

**Implementado na v1.2** (as quatro extensões que ficaram registradas na §9
da v1.1, todas opt-in, com o fio default bit a bit v1.0/v1.1):

1. **Confidencialidade e MAC por frame via TLS** — TLS 1.3 (rustls, `ring`)
   com certificado autoassinado + **pinning SHA-256** (§7): rustls não expõe
   TLS-PSK (rustls/rustls#174), então o PSK-HMAC da v1.1 (§4.6) continua
   disponível e combinável com TLS como camada de autorização de aplicação.
2. **Dicionário de compressão compartilhado entre frames** — derivado do
   registro do servidor (§4.8, id 2), gatilho no `HELLO`, zero bytes de
   dicionário no fio.
3. **IPv6 e SSM para o beacon** — grupos IPv6 (join com scope) e SSM IPv4
   (assinatura por fonte, RFC 4607) no §4.9.
4. **mDNS/DNS-SD** — §4.10, feature `mdns` default-off.

**Implementado na v1.3** (as quatro extensões que ficaram registradas na §9
da v1.2, todas opt-in, com o fio default bit a bit v1.0/v1.1/v1.2):

1. **SSM IPv6 para o beacon** — join source-specific em IPv6 com
   `MCAST_JOIN_SOURCE_GROUP` (RFC 3678/4604; §4.9): a API de socket que
   faltava existe via `setsockopt` bruto no Linux (a crate `socket2` não
   expõe a opção v6 — decisão registrada no relatório v1.3); em SO sem a
   opção, erro honesto "multicast indisponível". Sintaxe
   `[ff35::7080]:porta@[fe80::1%2]`; fonte da mesma família do grupo.
2. **TOFU como alternativa operacional ao pinning** (§7): endpoint
   `tcps:host:porta@tofu` + store JSON determinístico e atômico; primeira
   use grava, demais verificam, divergência ⇒ falha fechada com motivo no
   Caderno. Pin (`@sha256:HEX`) continua sendo o modo de maior garantia.
3. **zstd com dicionário treinado (id 3)** (§4.8): bit `CAPS` 4 (`ZSTD`,
   sempre com `DICT`), COVER sobre os nomes canônicos ordenados, nível 3,
   teto de dicionário 16 KiB, zero bytes de dicionário no fio; divergência
   de versão de zstd ⇒ `DecompressionFailed` (honesto). Razão medida:
   6,8× contra 5,6× do id 2 no payload canônico do bench.
4. **Resumo de sessão + 0-RTT TLS** (§7): `ClientConfig` cacheado por chave
   de confiança ⇒ segunda conexão retoma (`Resumed`); frame `CAPS`
   idempotente viaja como 0-RTT com teto de 512 B; sem aceitação, renegocia
   normal. `ACT`/`READ` nunca viajam adiantados (replay); com AUTH+PSK a
   ordem do §4.6 preserva o servidor-fala-primeiro.

**Implementado na v1.4** (as cinco extensões que ficaram registradas na §9
da v1.3, todas opt-in, com o fio default bit a bit v1.0/v1.1/v1.2/v1.3):

1. **TOFU estrito (`accept-new`)** (§7): endpoint `@tofu-estrito`; o alvo
   precisa preexistir no store — nunca aprende, nunca pergunta. Store lê
   legado/novo/misto; entrada só com o pin desconhecido ⇒ falha fechada com
   motivo (§7).
2. **Rotação de pins com sobreposição** (§7): multi-pin
   `@sha256:H1,H2` (teto 8), store multi-pino com
   `adicionar_pin`/`remover_pin`, e2e de rotação com certificado novo
   aceito durante a janela de sobreposição.
3. **zstd com dicionário versionado no fio (id 4 + `DICT_SYNC`)** (§4.8):
   bit `ZSTD_V` (5, sempre com `ZSTD + DICT`); troca de
   `(zstd_version, hash do dict)` após o `HELLO`; hash casado libera o id
   4 nos dois sentidos; divergente ⇒ degradação honesta para o id 3 com
   evento `fxp_dict_divergente` no Caderno — pontas com libzstd diferentes
   agora NEGOCIAM compatibilidade em vez de quebrar no primeiro frame.
4. **Sessão retomada entre processos** (§7): storage de sessões do
   SERVIDOR em disco (`--tls-sessions`, `CacheSessoesDisco`) — write-through
   atômico `0600`, evicção LRU-por-idade (teto 1024), poda de 7 dias;
   retomada `Resumed` + 0-RTT aceito contra um servidor RECOMEÇADO (e2e).
   Ticket do cliente em disco segue bloqueado no rustls 0.23 (§7 —
   rustls/rustls#2287, resolvido na 0.24).
5. **Benchmark de 0-RTT com RTT real** (§9): proxy TCP que injeta atraso
   unilateral por voo (`FXP_BENCH_RTT_US`, default 3000 µs ⇒ RTT 6 ms);
   0-RTT ≈ 22,4 ms × retomado sem 0-RTT ≈ 25,5 ms × plano ≈ 6,8 ms — o
   0-RTT poupa ~1 voo por conexão no RTT medido (números completos e
   método em [FXP-V1.4-REPORT](reports/FXP-V1.4-REPORT.md)).

**Fica registrado como trabalho futuro (fora da v1.4):**

1. **Ticket de sessão do CLIENTE em disco** — desbloqueado quando o
   workspace adotar rustls ≥ 0.24 (PR rustls#2907 expõe a serialização de
   `Tls13ClientSessionValue`; hoje o cliente retoma só dentro do mesmo
   processo). Com ele, o mesmo desenho do `CacheSessoesDisco` cobre a
   ponta cliente (arquivo `0600` no state dir do usuário).
2. **Prompt interativo de TOFU** (estilo `known_hosts` do SSH com
   confirmação humana) — segue fora do escopo do FXP (o bus não tem TTY);
   o TOFU estrito da v1.4 cobre o caso operacional por allow-list.
3. **`DICT_SYNC` com rollback de dicionário** — hoje o hash casado libera o
   id 4 para a CONEXÃO; uma renegociação de registro (HELLO novo) com dict
   diferente já é coberta pela re-derivação na conexão seguinte; sincronizar
   o dict no MEIO da conexão (sem reabrir) continuaria exigindo estado de
   compressão por conexão no codec — fora do princípio "codec stateless".
4. **Beacon autenticado** — o anúncio multicast segue sem MAC (o pin/TOFU
   do `tcps` é a raiz de confiança); assinar o beacon (ed25519 no TXT)
   continua registrado como ideia, não agendado.
