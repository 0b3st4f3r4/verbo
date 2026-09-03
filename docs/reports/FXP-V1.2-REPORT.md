# FXP v1.2 — Relatório de implementação (PLAN §8 item 9)

**Status:** implementado e verificado nesta janela. As quatro extensões
registradas na §9 da v1.1 — confidencialidade/MAC por frame via TLS,
dicionário de compressão compartilhado, beacon IPv6/SSM e mDNS/DNS-SD —
estão no contrato canônico ([FXP-SCHEMA-v1](../FXP-SCHEMA-v1.md) promovido a
**v1.2**). Invariantes mantidos: fio default **bit a bit v1.0/v1.1** (golden
bytes intactos), todo recurso novo **negociado e opt-in**, falha de recurso
desconhecido **fail-closed**, degradação honesta registrada no Caderno.

## 1. TLS 1.3 com pinning (`tcps`, §7)

- **Decisão de projeto (verificada na fonte):** rustls não expõe TLS-PSK
  (issue rustls/rustls#174, aberta). O modelo v1.2 é **certificado
  autoassinado + pinning por impressão digital**: endpoint
  `tcps:host:porta@sha256:HEX` (64 hex = SHA-256 do DER). O cliente rejeita
  handshake cujo certificado não bate **exatamente** com o pin — sem
  fallback para texto plano, em nenhuma condição.
- Provedor criptográfico `ring` (não aws-lc-rs: evita dependência de cmake;
  não rustcrypto: TLS completo em alpha). TLS 1.3 only; handshake com
  timeout próprio de 2 s (§6).
- CLI: `vbl fxpd --tls-cert CADEIA --tls-key CHAVE` (par obrigatório; erro
  de carga é honesto no arranque). E2E: pin certo conecta e lê; pin errado
  **succeed no `vbl run` com alerta honesto** (`sensor_inaccessible` com
  motivo `tls` no Caderno) — a honestidade do runtime não vira mentira de
  exit-code.
- Unix + TLS é recusado no arranque (empilhar camadas não faz sentido);
  peer plano diante de cliente TLS falha nas duas direções (cross-tests).

## 2. Dicionário de compressão compartilhado (§4.8, id 2)

- **Zero bytes de dicionário no fio.** O dicionário é derivado
  deterministicamente do registro do **servidor**: nomes canônicos
  ordenados, concatenados com `\n`, teto 64 KiB. Servidor usa o próprio
  registro; cliente deriva da resposta `HELLO` — que já era obrigatória na
  ordem v1.1 do handshake (§4.5).
- Máquina de estado (as duas pontas): `CAPS_OK` com bit `DICT` ⇒ `HELLO` vira
  gatilho; a resposta `HELLO` **nunca** sai comprimida com dict; o servidor
  só marca o dicionário pronto **depois** de receber o `HELLO` do cliente;
  frames de trabalho id 2 só partem dos dois lados após o gatilho.
- Fail-closed compatível: decoder sem dict diante do id 2 ⇒
  `UnknownCompression { received: 2 }` — o **mesmo** erro que um codec v1.1
  real produz (a promoção do bit 3 de CAPS é segura por construção:
  decoder v1.1 ignora bits reservados no decode).
- Medidas (bench `v12_tls_dict`, `--quick`, Ryzen 7 7735HS, payload
  sintético de 40 leituras em lote com nomes canônicos de ~47 B):

| Medida | Plano (id 1) | Dict (id 2) | Δ |
|---|---|---|---|
| frame em lote repetitivo | 423 B | 365 B | **−13,7 %** |
| encode | 1,46 µs | 1,82 µs | +0,36 µs/frame |
| decode | — | 2,04 µs | — |

  O ganho do dict cresce com nomes **novos** no frame (que o LZ4 simples não
  consegue deduplicar); no payload repetitivo acima o id 1 já comprime 5,4×
  sozinho e o dict soma 13,7 %. Custo de CPU: +25 % no encode — orçamento
  confortável contra o teto de 10 ms p95 de leitura remota.

## 3. Beacon IPv6 e SSM (§4.9)

- **IPv6:** grupos `[ff15::7080]:porta` (com scope numérico opcional para
  link-local); join `join_multicast_v6` com o scope do grupo; hops v6 = TTL
  v4 do §4.9 (o `std` não expõe hops v6 — via `socket2`).
- **SSM IPv4** (RFC 4607): config `239.255.70.81:7080@192.168.1.10` assina
  (fonte, grupo) — sem ruído de outros `fxpd`; o servidor anuncia com bind
  local explícito para que a FONTE do datagrama seja o IP assinado
  (`Announcer::start_bound`).
- **Fora do escopo (honesto):** SSM **IPv6** — nem `std` nem `socket2`
  expõem source-specific join para v6; registrado na §9 da v1.2 como
  trabalho futuro.
- O datagrama FXPD é **intocado** (versão 1, mesmos campos) — a extensão é
  só na camada de socket. Testes ao vivo rodam no loopback do host; em rede
  sem multicast são skip gracioso (mesmo padrão da v1.1).
- Config: `discover_group` no barramento (parse honesto; grupo ruim ⇒
  registrado porém inacessível com motivo, nunca erro de construção).

## 4. mDNS/DNS-SD (§4.10, feature `mdns` default-off)

- Serviço `_fxp._tcp.local.` via `mdns-sd` 0.21 (puro Rust, thread própria,
  sem runtime async). TXT: `id` (canônico para match), `hash` (mesma
  impressão digital do beacon §4.9) e, com TLS, `tls=1` + `pin` (hex
  SHA-256 do DER — o consumidor resolve direto para `tcps` com pin).
- Endpoint `mdns:<identificador>`: sem a feature o parse **rejeita** o
  endpoint com erro honesto (nada de aceitar e falhar depois); com a
  feature, resolve no `build()` do barramento, janela idêntica ao beacon.
- CLI: `vbl fxpd --announce-mdns ID` (idem — sem a feature, aviso honesto e
  anúncio NÃO ativo).
- mDNS é lossy como UDP: ausência de resposta não é recusa; sem cache além
  da janela (§4.10).

## 5. Verificação

- `vbl-fxp` (default): 152 testes — suites `schema_roundtrip` (29, inclui
  golden bytes v1.0 intactos e a atualização honesta do teste de bits
  reservados da v1.1 para os bits 4–15), `bus`, `transport`, `peer`, `bus_e2e`,
  `registry`, `discover`, `v11` e `v12` (10: 6 TLS + 4 dict).
- `vbl-fxp` (`--features mdns`): `v12` vai a 16 — +anúncio/resolve mDNS,
  TXT tls/pin, endpoint `mdns:` parse/e2e (mDNS do host de referência
  funcional; em CI sem mDNS são skip gracioso).
- `vbl-cli`: 44 unitários + 10 E2E, incluindo `e2e_fxpd_tls_pin_certo_
  conecta_e_errado_falha_fechada` (pin certo lê sem eventos de falha; pin
  errado segue a semântica honesta do PSK: `sensor_inaccessible` + motivo
  `tls` no Caderno).
- Benches novos no grupo `v12_tls_dict`: `tls_handshake_ler`,
  `tcp_plano_handshake_ler`, `encode_lz4_simples`, `encode_lz4_dict`,
  `decode_lz4_dict`. O handshake TLS não se separou do TCP plano no ruído
  deste bench (ambos ~5,1 ms por conexão nova na máquina de referência —
  custo dominado pelo caminho de conexão do bench; frame estabelecido não
  muda de orçamento).

## 6. O que ficou para depois (§9 da v1.2)

SSM IPv6 (aguarda API de socket), TOFU como alternativa operacional ao
pinning, zstd com dicionário treinado (ids ≥ 3), 0-RTT/resumo de sessão TLS.
