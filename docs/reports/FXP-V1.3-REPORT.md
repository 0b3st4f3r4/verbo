# FXP v1.3 — Relatório de implementação (PLAN §8 item 10)

**Status:** implementado e verificado nesta janela. As quatro extensões
registradas na §9 da v1.2 — SSM IPv6, TOFU como alternativa operacional ao
pinning, zstd com dicionário treinado (id 3) e resumo de sessão + 0-RTT TLS
— estão no contrato canônico ([FXP-SCHEMA-v1](../FXP-SCHEMA-v1.md) promovido
a **v1.3**). Invariantes mantidos: fio default **bit a bit
v1.0/v1.1/v1.2** (golden bytes intactos), todo recurso novo **negociado e
opt-in**, falha de recurso desconhecido **fail-closed**, degradação honesta
registrada no Caderno.

## 1. SSM IPv6 para o beacon (§4.9)

- **Decisão de projeto (verificada na fonte):** nem `std` nem `socket2` 0.6
  expõem source-specific join para IPv6 (só `join_ssm_v4`). A API que
  faltava existe no Linux como opção de socket bruta:
  `setsockopt(fd, IPPROTO_IPV6, MCAST_JOIN_SOURCE_GROUP, …)` (RFC 3678/4604)
  com `struct group_source_req` — a libc-rs liga a constante mas NÃO o
  struct, então o struct é declarado localmente (`#[repr(C)]`,
  `sockaddr_storage` preenchido por cópia do `sockaddr_in6` + `scope_id`).
  Escopo do código: `#[cfg(target_os = "linux")]`; em outros SO o erro é
  honesto ("multicast indisponível neste host") — nada de adivinhar números
  de opção que diferem entre BSDs.
- Sintaxe do config: `[ff35::7080]:porta@[fe80::1%2]` — fonte v6 exige
  colchetes e scope **numérico** (`%N`; o arquivo de config não é `/proc`
  para resolver nome de interface). Fonte da família errada (v6 em grupo v4
  ou vice-versa) ⇒ erro de parse honesto (SSM é mesma-família).
- API: `discover::FonteSsm { V4(Ipv4Addr), V6 { addr, scope } }` em
  `parse_group`/`discover_peers_ssm`/`listener_multicast`; o datagrama FXPD
  continua **intocado** (a extensão é só na camada de socket).
- Testes: parse de fonte escopada, recusas honestas e beacon SSM v6 ao vivo
  no loopback (`tests/v13.rs`); o antigo teste de "SSM v6 fora do escopo" da
  v1.2 agora afirma a recusa de família (o resto virou feature).

## 2. TOFU como alternativa operacional ao pinning (§7)

- **Modelo:** endpoint `tcps:host:porta@tofu` — o operador não copia pin
  para o config; a impressão digital SHA-256 do DER vista na **primeira**
  conexão é gravada num store local e as seguintes verificam contra ela.
  Divergência (certificado diferente no mesmo host:porta) ⇒ **falha
  fechada** com motivo TOFU no Caderno (`sensor_inaccessible` + "tofu").
  Semântica `known_hosts` do SSH; sem TOFU estrito (`accept-new`) nem prompt
  interativo — quem quer pin imutável continua com `@sha256:HEX` (v1.2,
  intocada). Registry: slot único de confiança `Trust::{Pin, Tofu}`;
  `@confiar`/qualquer outro slot ⇒ erro de parse honesto.
- **Store:** JSON determinístico (chaves ordenadas) `{"host:porta":
  "sha256:hex64"}` — o algoritmo vai no arquivo (autodescritivo); hex puro
  do formato inicial também carrega. Escrita **atômica** (`.json.tmp` +
  rename). Caminho: flag `--tofu-store ARQUIVO` do `vbl run`; default
  `$XDG_STATE_HOME` (ou `~/.local/state`) + `/verbo/fxp-known-hosts.json`.
  Sem caminho nenhum + endpoint `@tofu` ⇒ falha fechada da conexão (nunca
  confiar sem poder registrar). Store corrupto ⇒ erro de abertura com
  motivo, nunca lixo parcial.
- **Compartilhamento:** o bus abre o store uma única vez (primeira conexão
  `@tofu`) e o compartilha (`Arc<Mutex<…>>`) — a "primeira use" grava
  exatamente uma vez mesmo com múltiplos dispositivos do mesmo peer.
- Verificador `VerificadorTofu` implementa `ServerCertVerifier` do rustls:
  divergência vira `Error::General("TOFU (host:porta): … — conexão recusada
  (fail closed, v1.3 §7)")`; assinaturas validadas pelos helpers `ring` do
  rustls (mesmo nível de verificação do pinning v1.2).
- CLI e2e (`e2e_fxpd_tofu_primeira_uso_grava_divergencia_falha_fechada`):
  1ª rodada contra daemon legítimo conecta, lê sem alerta e grava o store;
  2ª rodada na MESMA porta com cert intruso diverge ⇒ run honesto (sem
  crash), `sensor_inaccessible` + motivo `tofu` no Caderno.

## 3. zstd com dicionário treinado (§4.8, id 3, bit `ZSTD`)

- **Fio:** bit 4 de `CAPS` (`ZSTD`, reservado na v1.2) e algoritmo `id 3`
  no byte reservado do header. `ZSTD` é negociado **sempre junto** com
  `DICT` — o gatilho do `HELLO` é o mesmo; sem concessão de zstd a
  degradação cai no id 2 pela interseção de `CAPS_OK` (nunca silenciosa).
  Bits reservados de `CAPS` passam a ser 5–15 (teste da v1.2 atualizado
  honestamente: bit 4 agora encoda limpo).
- **Dicionário treinado:** a MESMA matéria do id 2 (nomes canônicos do
  registro do servidor, ordenados) vira **amostras** do treino COVER
  (`zstd::dict::from_samples`), teto **16 KiB**, nível do fio fixo **3**
  (constante da especificação). Zero bytes de dicionário no fio; a
  derivação acontece na fase de handshake, que ganhou prazo próprio
  (`max(timeout, 500 ms)` — §6) porque treinar é trabalho real (~5 ms no
  registro do bench), não cabe no prazo de leitura de 10 ms.
- **Honestidade do treino:** determinístico para (nomes, versão do zstd) —
  pontas com versões diferentes podem derivar dicionários diferentes ⇒
  `DecompressionFailed` (fail closed). Registro pequeno demais para o COVER
  (poucas dezenas de bytes de amostra) ⇒ treino falha ⇒ o servidor **não
  concede** `ZSTD` (o bit sai da interseção — degradação explícita, testada).
- **Fail-closed tipado:** o dicionário da conexão carrega o algoritmo
  (`DictConexao::{Lz4, Zstd}`): id 2 só decodifica com a matéria
  concatenada, id 3 só com a treinada — o contrário é
  `UnknownCompression{2}/{3}`, exatamente o que codecs v1.1/v1.2 produzem
  (promoção do bit 4 segura por construção, como a do bit 3 na v1.2).
- **Dependência C (reversão deliberada da v1.1):** `zstd` 0.13 (bindings
  sobre a lib C) entra no workspace com justificativa: o treino de
  dicionário não existe em crate pura-Rust madura; a v1.1 recusou zstd
  porque "não justificam dep C" — com o dicionário treinado no contrato,
  justifica (mesmo critério da v1.2 para `rcgen`/`rustls`).
- **Medidas** (bench, `--quick`, payload canônico do bench: lote de 40
  leituras, 2014 B planos; Ryzen 7 7735HS):

| Medida | id 2 (LZ4+dict) | id 3 (zstd+dict treinado) | Δ |
|---|---|---|---|
| frame no fio | 361 B (5,6×) | **298 B (6,8×)** | −17,5 % de bytes |
| encode | ~8,2 µs | 10,2 µs | +2,0 µs/frame |
| decode | ~5,9 µs | 6,8 µs | +0,9 µs/frame |
| dicionário derivado | 1639 B (concat) | 1321 B (treinado) | — |
| treino (uma vez por handshake) | — | ~5,25 ms | §6 |

  Orçamento confortável contra o teto de 10 ms p95 de leitura remota
  (compressão é por frame de resposta, treino é por handshake).

## 4. Resumo de sessão + 0-RTT TLS (§7)

- **Resumo:** o `ClientConfig` do cliente é cacheado por chave de confiança
  (`OnceLock` + `Mutex<BTreeMap>`: pin ou `host:porta@tofu`) — o rustls só
  retoma sessão reusando o MESMO `Arc<ClientConfig>`, então o cache é o que
  FAZ a retomada funcionar (`Resumption::in_memory_sessions(256)`, default
  dos dois lados). Teste: 1ª conexão `HandshakeKind::Full`, segunda
  `Resumed` contra o mesmo peer.
- **0-RTT:** `enable_early_data` no cliente + `max_early_data_size = 512`
  no servidor (`EARLY_DATA_MAX`). O frame `CAPS` (idempotente por conexão)
  parte JUNTO do `ClientHello`; aceito ⇒ o `CAPS_OK` chega no primeiro voo
  (o cliente não renegocia — `Connection::negotiate` consome a marca de
  0-RTT); recusado/sessão nova ⇒ o frame não foi entregue e o cliente
  **renegocia normalmente** (degradação honesta, testada: sem
  `early_caps`, conexão retomada segue o caminho normal e funciona).
- **Replay (honesto no contrato):** 0-RTT é replayável por atacante de
  rede; o ÚNICO frame adiantado é o `CAPS` idempotente — `ACT`/`READ`
  nunca viajam adiantados (exigem handshake completo, 1-RTT). Com
  AUTH+PSK o cliente não adianta `CAPS` nenhum: o servidor fala primeiro
  (`AUTH_CHALLENGE`, §4.6) — a ordem v1.1 fica intacta.
- **Bench honesto:** com o cache v1.3, o antigo `tls_handshake_ler` da v1.2
  passou a medir a retomada — renomeado para
  `tls_handshake_resumido_ler` (o número de handshake completo da v1.2
  fica no relatório v1.2 como base). Novo grupo `v13_tls_0rtt`:
  `tls_retomado_caps_negociado_ler` × `tls_retomado_caps_0rtt_ler`
  (isolam o ganho do 0-RTT — no loopback do bench ambos ficam dentro do
  ruído do custo fixo de conexão do harness, ~5,1 ms; o ganho real é de
  1 RTT por conexão e aparece em rede com RTT > 0, registrado como
  trabalho futuro de medição).

## 5. Verificação

- `vbl-fxp` (default): 185 testes — inclui as 12 novas situações de
  `tests/v13.rs` (SSM v6, resumo Full→Resumed, 0-RTT aceito e renegociação
  honesta, store TOFU: grava/recarrega/corrupto/e2e de divergência com
  motivo no Caderno, registry `@tofu`/`@sha256:`, dict treinado:
  determinismo/id 3 roundtrip/fail-closed tipado/degradação sem treino) e
  o teste de bits reservados de `CAPS` atualizado (5–15, bit 4 limpo).
- `vbl-cli`: e2e novo `e2e_fxpd_tofu_primeira_uso_grava_divergencia_falha_
  fechada` (flag `--tofu-store` ponta a ponta contra dois daemons na mesma
  porta). Flags novas: `vbl run --tofu-store ARQUIVO`, `vbl run --zstd`,
  `vbl fxpd --zstd` (implica `--dict` no anúncio — o honesto, já que um
  bit sem o outro nunca seria concedido).
- Suites completas `vbl-fxp` + `vbl-cli` verdes; golden bytes v1.0
  intactos (`schema_roundtrip`).

## 6. O que ficou para depois (§9 da v1.3)

TOFU estrito (`accept-new`) e rotação de pins com sobreposição; zstd com
dicionário versionado no fio (id 4+, para pontas com versões de zstd
diferentes negociarem compatibilidade); key rotation do certificado com
overlap de pin; sessão retomada entre processos (tickets em disco — hoje o
cache é em memória por processo); medição do ganho do 0-RTT em rede com
RTT real (> 1 ms).
