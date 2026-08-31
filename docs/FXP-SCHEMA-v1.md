# FXP — Schema de Mensagem v1

**Status:** canônico para a Etapa 3 (PLAN §3.5). Definido **antes** dos drivers,
como exige o entregável. Implementação de referência: `nucleo/crates/vbl-fxp/src/schema.rs`
(testes de roundtrip em `tests/schema_roundtrip.rs`).

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

## 2. Enquadramento (frame)

```
┌──────────────┬─────────────────────────────┐
│ length: u32  │ payload (length bytes)      │
│ LE, exclui   │ header (12 B) + corpo       │
│ o próprio u32│                             │
└──────────────┴─────────────────────────────┘
```

- `length` ≤ **8192** (guarda anti-inchaço; frame máximo = 8196 bytes no fio).
- `length` < 12 ou payload truncado ⇒ **erro de decodificação** (nunca
  mensagem parcial silenciosa).
- Demais violações (magic, versão, opcode desconhecido, campo faltante,
  UTF-8 inválido, `NaN`/`±∞`) ⇒ erro de decodificação com razão explícita.

## 3. Header (12 bytes, fixo)

| Offset | Campo        | Tipo       | Valor/Significado                                   |
|-------:|--------------|------------|-----------------------------------------------------|
| 0      | `magic`      | `[u8;3]`   | `"FXP"` (0x46 0x58 0x50)                             |
| 3      | `version`    | `u8`       | `1` — decodificador rejeita versão desconhecida      |
| 4      | `opcode`     | `u8`       | tabela §4                                            |
| 5      | `flags`      | `u8`       | tabela §5                                            |
| 6      | `reservado`  | `u8`       | `0` no encode; ignorado no decode (futuro)           |
| 7      | `name_len`   | `u8`       | 0–255; nome simbólico (0 quando a op não tem nome)   |
| 8      | `seq`        | `u32` LE   | id de correlação ack (wrapping)                      |

Após o header vem `name` (`name_len` bytes UTF-8; nome simbólico do sensor ou
ator — o alias é permitido e o canônico viaja na resposta quando aplicável) e,
quando a op define, o corpo específico.

## 4. Opcodes

| Valor | Nome               | Direção        | Corpo após `name`                                       |
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

## 5. Flags (u8)

| Bit | Nome            | Significado                                                |
|----:|-----------------|------------------------------------------------------------|
| 0   | `FLAG_ACK`      | remetente exige resposta com o mesmo `seq`                 |
| 1   | `FLAG_ERRO`     | resposta de erro (`READ_ERR`, ack negativo)                |
| 2   | `FLAG_FALLBACK` | entrega efetivada por ator alternativo (FORMAL §4.3)       |
| 3   | `FLAG_SINTETICO`| dado de origem simulada (`measurement_status`) — §4.7      |
| 4–7 | reservados      | `0` no encode; ignorado no decode                          |

## 6. Timeouts e políticas (defaults da Etapa 3)

| Parâmetro               | Default     | Orçamento (AGENTS §1.2 EIF)                     |
|-------------------------|-------------|--------------------------------------------------|
| `read_timeout` (fio)    | 10 ms       | leitura local ≤ **1 ms** p95; remota ≤ **10 ms** p95 |
| `act_timeout` local     | 50 ms       | ack local ≤ **50 ms** p95                        |
| `act_timeout` remoto    | 500 ms      | ack remoto ≤ **500 ms** p95                      |
| `cache_ttl`             | 100 ms      | cache de leitura real (PLAN §3, mitigação)       |
| `retries` (transporte)  | 1           | PLAN §3: fila com retry e fallback               |
| `queue_timeout`         | 2 ticks     | comando pendente expira com evento no Caderno    |

Timeouts de leitura/ack medem **tempo de parede** e se aplicam ao transporte
com fio (Unix/TCP); a fronteira in-process é chamada direta (orçamento
≤ 10 µs/mensagem). A fila de retry avança no **relógio virtual** (`on_tick`)
para manter os testes determinísticos em CI.

Política de entrega de `act` no bus:

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

O servidor de referência (testes/integração) fala exatamente o frame §2;
nenhuma mensagem v1 é transport-specific.

## 8. Matriz opcode × mensagem obrigatória (rastreabilidade)

| Requisito (PLAN/FORMAL) | Mensagem v1                        | Teste |
|--------------------------|------------------------------------|-------|
| serializa/desserializa sem perda (AGENTS Etapa 3) | todas | `tests/schema_roundtrip.rs` |
| falha de sensor nunca é 0.0 (§4.7) | `READ_ERR` | `tests/schema_roundtrip.rs` |
| dado sintético marcado (§4.7) | `FLAG_SINTETICO` | `tests/schema_roundtrip.rs` |
| limites inclusivos (§4.3) | `ACT_ACK.Rejeitado` | `tests/bus.rs` |
| fallback do registro (§4.3) | `FLAG_FALLBACK` + `FallbackExecutado` | `tests/bus.rs` |
| heartbeat (BDD Caso 3) | `HEARTBEAT`/`HEARTBEAT_ACK` | `tests/transport.rs` |
| aliases com nome canônico (§6) | `READ_OK.canonical` | `tests/registry.rs` |
| prioridade pós-subvert (§4.5) | fila prioritária | `tests/queue.rs` |

## 9. Extensões futuras (fora do v1)

Compressão, batching de leituras, descoberta multicast, autenticação do canal
remoto e timestamps absolutos no fio (o Caderno usa o relógio virtual do
runtime — timestamp físico é atribuição de laboratório, Etapa 5).
