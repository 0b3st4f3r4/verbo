# Changelog

Todas as mudanças notáveis do VerboLang ficam registradas neste arquivo. O
formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e o
processo/calendário de releases está em
[`docs/RELEASES.md`](docs/RELEASES.md): `vYYYY.N` — 6 meses de P&D + 6 anos
de suporte + 6 meses de descontinuação = 7 anos de ciclo de vida.

## [Não lançado]

### Adicionado
- **FXP v1.4** — as cinco extensões remanescentes da §9 da v1.3
  (`docs/FXP-SCHEMA-v1.md` promovido a v1.4; fio default continua byte a
  byte v1.0/v1.1/v1.2/v1.3, golden bytes intactos; tudo negociado e opt-in):
  - **TOFU estrito (`accept-new`, §7)** — endpoint
    `tcps:host:porta@tofu-estrito`: o alvo precisa JÁ existir no store
    (allow-list operacional; nunca aprende, nunca pergunta); alvo ausente ⇒
    `TofuFalha::Desconhecida` e conexão recusada com motivo. Store lê os
    formatos legado, novo (`{"pins":[…]}`) e a mistura.
  - **Rotação de pins com sobreposição (§7)** — multi-pin
    `tcps:host:porta@sha256:H1,H2` (teto 8): adicione o pin novo, troque o
    certificado, remova o velho — clientes com pin duplo não caem na
    janela; `adicionar_pin`/`remover_pin` no store; roundtrip de config
    pelo `description()`.
  - **zstd com dicionário versionado no fio (§4.8, id 4)** — bit `CAPS`
    `ZSTD_V` (5, sempre com `ZSTD + DICT`); novo handshake `DICT_SYNC`
    troca `(versão do zstd, SHA-256 do dict treinado)` após o `HELLO`;
    hash casado libera o id 4 nos dois sentidos; divergente (libzstd de
    versões diferentes nas pontas) ⇒ degradação honesta para o id 3 com
    evento `fxp_dict_divergente` no Caderno — pontas agora NEGOCIAM
    compatibilidade em vez de quebrar no primeiro frame.
    `vbl run --zstd-v`; `vbl fxpd --zstd-v` (implica `--zstd`).
  - **Sessão retomada entre processos (§7)** — cache de sessões do
    SERVIDOR em disco: `vbl fxpd --tls-sessions ARQUIVO` (`CacheSessoesDisco`,
    write-through atômico `0600`, evicção por idade, teto 1024) — o `fxpd`
    que renasce recarrega os blobs e clientes vivos retomam (`Resumed`) com
    0-RTT intacto. Ticket do CLIENTE em disco segue bloqueado no rustls
    0.23 (rustls/rustls#2287; resolvido na 0.24 — trabalho futuro).
  - **Benchmark de 0-RTT com RTT real (§9)** — grupo `v14_tls_0rtt_rtt` no
    bench: proxy TCP injeta atraso unilateral (`FXP_BENCH_RTT_US`, default
    3000 µs ⇒ RTT 6 ms); 0-RTT ≈ 22,4 ms × retomado sem 0-RTT ≈ 25,5 ms ×
    plano ≈ 6,8 ms — ~1 voo poupado por conexão
    ([FXP-V1.4-REPORT](docs/reports/FXP-V1.4-REPORT.md)).
- **FXP v1.3** — as quatro extensões remanescentes da §9 da v1.2
  (`docs/FXP-SCHEMA-v1.md` promovido a v1.3; fio default continua byte a
  byte v1.0/v1.1/v1.2, golden bytes intactos; tudo negociado e opt-in):
  - **SSM IPv6 (§4.9)** — assinatura de fonte em grupos IPv6
    (`[ff35::7080]:porta@[fe80::1%2]`) via `MCAST_JOIN_SOURCE_GROUP`
    (RFC 3678/4604, `setsockopt` bruto no Linux — socket2 não expõe a opção
    v6; em outros SO, erro honesto de multicast indisponível).
  - **TOFU (§7)** — alternativa operacional ao pinning: endpoint
    `tcps:host:porta@tofu` grava a impressão digital da primeira conexão
    num store JSON atômico (`vbl run --tofu-store`; default
    `~/.local/state/verbo/fxp-known-hosts.json`) e as seguintes verificam
    contra ela — divergência **fecha a conexão** com motivo TOFU no
    Caderno. Pin `@sha256:HEX` da v1.2 intocado.
  - **zstd com dicionário treinado (§4.8, id 3)** — bit `CAPS` `ZSTD`
    (sempre negociado com `DICT`); COVER sobre os nomes canônicos do
    registro (teto 16 KiB, nível 3), zero bytes de dicionário no fio;
    6,8× de compressão contra 5,6× do id 2 no payload canônico; registro
    pequeno demais para treinar ⇒ `ZSTD` sai da interseção (degradação
    honesta). `vbl run --zstd`; `vbl fxpd --zstd` (implica `--dict`).
  - **Resumo de sessão + 0-RTT TLS (§7)** — `ClientConfig` cacheado por
    confiança ⇒ segunda conexão retoma (`Resumed`); o frame `CAPS`
    idempotente pode partir como 0-RTT (teto 512 B) junto do
    `ClientHello` — recusado, o cliente renegocia normal. `ACT`/`READ`
    nunca viajam adiantados; com AUTH+PSK nada é adiantado (servidor
    fala primeiro).
- **FXP v1.2** — as quatro extensões remanescentes da §9 da v1.1
  (`docs/FXP-SCHEMA-v1.md` promovido a v1.2; fio default continua byte a
  byte v1.0/v1.1, golden bytes intactos; tudo negociado e opt-in):
  - **TLS 1.3 (`tcps`, §7)** — confidencialidade + MAC por frame via
    `rustls` (provedor `ring`); rustls não expõe TLS-PSK (#174), então o
    modelo é certificado autoassinado + **pinning SHA-256 do DER** no
    endpoint `tcps:host:porta@sha256:HEX` — certificado sem o pin exato
    **fecha a conexão** (nunca degrada para texto plano). `vbl fxpd
    --tls-cert/--tls-key` (par obrigatório); Unix+TLS é recusado no arranque.
  - **Dicionário de compressão compartilhado (§4.8, id 2)** — derivado do
    registro do servidor (nomes canônicos ordenados + `\n`, teto 64 KiB);
    o `HELLO` vira gatilho do handshake e **nenhum byte de dicionário cruza
    o fio**; decoder sem dict vê id 2 como v1.1 (fail-closed idêntico).
    `compression_dict = true` no cliente; `vbl fxpd --dict` no servidor.
  - **Beacon IPv6 + SSM (§4.9)** — grupos IPv6 (`[ff15::7080]:porta`, scope
    numérico p/ link-local) e SSM IPv4 (RFC 4607, `ip:porta@fonte`); o
    datagrama FXPD é intocado. SSM IPv6 fora do escopo (§9, aguarda API).
  - **mDNS/DNS-SD (§4.10)** — feature `mdns` default-off (`mdns-sd`):
    `_fxp._tcp.local.` com TXT `id`/`hash` (+`tls`/`pin`); endpoint
    `mdns:ID` e `vbl fxpd --announce-mdns ID`; sem a feature, parse/flag
    rejeitam com erro honesto.
- **FXP v1.1** — as cinco extensões do fio registradas como futuras no §9 do
  schema (`docs/FXP-SCHEMA-v1.md`, agora v1.1; o fio padrão continua byte a
  byte o do v1.0, com teste de bytes-fixos na suíte):
  - **CAPS/CAPS_OK (§4.5)** — negociação de capacidades na abertura da
    conexão, interseção dos bits; recurso só existe se os dois lados
    anunciarem (`vbl_fxp::schema::caps`).
  - **Autenticação PSK (§4.6)** — handshake `AUTH_CHALLENGE`/`AUTH_RESPONSE`/
    `AUTH_OK` com HMAC-SHA256 sobre `"FXP-AUTH1" ‖ nonce_cliente ‖
    nonce_servidor` (nonces de 32 B por conexão, verificação em tempo
    constante); cliente `vbl run --fxp-psk-env VAR`, servidor
    `vbl fxpd --auth psk:VAR` — a chave só entra por env, nunca por arquivo;
    chave errada **fecha a conexão** (sem degradação).
  - **READ_BATCH (§4.7)** — lote de 1..=64 leituras em 1 RTT;
    `batch_prefetch = true` no cliente prefetch todos os sensores vencidos do
    peer no primeiro cache-miss; item que falha no lote **não vira alerta** —
    o alerta continua pertencendo à pergunta do programa (honestidade §4.7).
  - **Compressão LZ4 (§4.8)** — corpos grandes comprimidos com `lz4_flex`
    (região < 512 B e blob que infla nunca viajam comprimidos; teto de 8192 B
    descomprimido = guarda contra bomba).
  - **FLAG_TIMESTAMP (§5)** — carimbo físico `fio_us` nas respostas
    (`wire_timestamp = true`); o Caderno continua no relógio virtual —
    timestamp físico é anotação de laboratório, não tick.
  - **Descoberta multicast (§4.9)** — beacon UDP `FXPD` no grupo
    `239.255.70.80:7080` (TTL 1, 2 s, opt-in): `vbl fxpd --announce ID`
    anuncia, `endpoint = discover:ID` resolve no build do barramento; sem
    anúncio no prazo ⇒ registrado porém inacessível.
- **`vbl fxpd`** — o servidor de referência do schema (§7): `--serve
  unix:PATH|tcp:PORTA`, recursos por flag (`--batch`/`--timestamp`/
  `--compress`/`--announce`), Caderno com `--ledger`; máquina de estados
  canônica em `vbl-fxp::peer::PeerServer` (AUTH → CAPS → trabalho).
- Config de texto v1.1 (docs/FXP-SCHEMA-v1.md §6): `batch_prefetch`,
  `compression`, `wire_timestamp`, `compress_threshold`; degradação v1.0
  automática com peer antigo (evento `fxp_peer_v1` no Caderno).
- Benches criterion v1.1 (`fxp_v11_*`) e relatório de medidas
  (`docs/reports/FXP-V1.1-REPORT.md`).

### Medido (máquina de referência, `cargo bench --quick`)
- Lote §4.7: ciclo de atualização de 8 sensores remotos cai de **117,4 µs**
  (8 RTTs) para **22,3 µs** (1 RTT + cache) — **5,3× mais rápido**.
- FLAG_TIMESTAMP §5: +5 ns por roundtrip de codec (113,5 → 119,6 ns).
- Handshake PSK+CAPS §4.6: ~4 µs sobre a conexão plana (custo do fio;
  latência total dominada pelo polling do loop de aceitação).
- LZ4 §4.8: HELLO de 60 dispositivos roundtrip 9,2 µs (comprimido) vs 7,9 µs
  (plano) — o ganho é banda (wire menor), não CPU.

### Corrigido
- **FXP v1.1 — colisão de tag no item de lote (§4.7)**: a razão 0
  (`nao_registrado`, §4.1) de um item de `READ_BATCH_OK` serializava como o
  byte de status 0 — que significa "ok" — e o cliente leria um valor
  fantasma, dessincronizando a conexão. A razão 0 agora viaja como tag 4
  (bytes de status 0..=3 preservados; achado pela varredura de truncamento
  do codec, que testa todos os prefixos de 13 mensagens).

## [v2027.0.0-alpha.0] — 2026-09-01

Primeiro pré-lançamento público da linha `v2027.0` (fase alpha: pesquisa,
experimentação e definição de escopo). Versão do workspace:
`2027.0.0-alpha.0`; tag de corte: `v2027.0.0-alpha.0`.

### Adicionado
- `site/` — livro de documentação didática em mdBook, publicado em
  **verbolang.org/docs** via GitHub Pages: trilha em sete capítulos
  (visão geral → instalação → conjugações → reviews → FXP → Caderno →
  receitas), seções de Referência (FORMAL, manifesto, cheat sheets, schemas,
  ADR) e de Projeto (README, PLAN, RELEASES, CHANGELOG) montadas dos
  próprios arquivos do repositório; tema da marca (vidro, aurora, horizonte
  tracejado, ciano da razão) com fontes Inter/Iosevka, realce `verbolang`
  próprio, Mermaid vendored e cromo nas 7 línguas da família. Montador
  `scripts/build_site.py` (reescrita de links com fallback canônico no
  GitHub e validação `--check`), alvos `make site-check`/`site-build`/`site`.
- Publicação no crates.io (RELEASES.md § crates.io): workflow
  `publish.yml` (tag `v*` + disparo manual com dry-run, ordem de dependência
  com espera de indexação, idempotente), metadados de pacote nos quatro
  crates (`repository`, `rust-version`, `readme`, `keywords`,
  `categories`), dependências internas com `version` + `path` no workspace
  e alvo `make rust-package` + passo de empacotamento no CI de push.
- `core/` — núcleo em Rust: parser (`vbl-lang`), motor de ticks
  (`vbl-runtime`), barramento FXP com registro de dispositivos (`vbl-fxp`)
  e Caderno de produção com cadeia SHA-256 (`vbl-cli`); matriz de testes,
  benches criterion e orçamentos de heap.
- `web/` — família de UI em vidro: dashboard (`index.html`), chat com modo
  "+ VerboLang" (`chat.html`), métricas ao vivo (`metrics.html`) e
  documentação renderizada (`docs.html` via `md.js`) — espectro da marca,
  horizonte tracejado, i18n em 7 línguas e badge de diagnóstico.
- `design/` — marca paramétrica: disco de vidro centrado no nó violeta
  (o verbo) com os três lados do triângulo (frio/energia/meta).
- `docs/` — especificação formal (FORMAL.md), manifesto, cheat sheets
  (completo e denso para agentes), plano de execução, ADRs e o processo de
  releases (RELEASES.md).
- `scripts/` — ponte do dashboard (`webui.py`), `serve-local-llm.sh`,
  Verbo Shell (`vsh.sh`) e validação do cheat sheet contra o LLM local.

### Alterado
- CI de push ganha o job `site` (portaria do livro: montagem + validação +
  `mdbook build` com mdbook 0.5 fixado); workflow novo `site.yml` publica o
  livro no GitHub Pages em push para main, em tag `v*` e manualmente; a
  release empacota o livro compilado (site/book) no tarball.
- Versão do workspace: `2027.0.0-alpha.0` (era `0.1.0-alpha.0`, nunca
  publicada) — a estreia pública adota a gramática da linha
  (`v2027.0.0-alpha.N`), conforme RELEASES.md § cargo/SemVer.
- Pre-commit migrado do script `.githooks/pre-commit` (158 linhas) para o
  framework [pre-commit](https://pre-commit.com): `.pre-commit-config.yaml`
  com os mesmos 12 estágios como hooks locais (`repo: local`, `make`), na
  ordem do CI, com os gates de cobertura ≥ 90% (pytest-cov e llvm-cov).
  `make hooks` aponta o `core.hooksPath` para um wrapper fino em
  `.githooks/pre-commit`, que só resolve o ambiente (`PRE_COMMIT_HOME` no
  workspace — HOME pode ser read-only, mesma regra do `CARGO_HOME`) e delega
  ao framework. Modo rápido: `make pc ARGS="run estaticos tdd-cobertura bdd
  clippy testes e2e"` (equivale ao antigo `VBL_PRE_COMMIT=quick`);
  `SKIP=<ids>` pula estágios. Novos alvos: `make test-cov`, `make
  rust-fxp-probe` e `make pc`; `make rust-bench` aceita `BENCH_ARGS`
  (ex.: `BENCH_ARGS=--quick`). Dependência `pre-commit>=4.0` em
  `requirements-dev.txt`.

### Corrigido
- Gate de cobertura Rust (llvm-cov ≥ 90%) era **dependente do host**: a
  auto-descoberta do FXP (`drivers::discover*`) sondava o `/sys` real, e a
  cobertura variava entre a máquina de referência (AMD, com k10temp/RAPL
  reais) e a VM do CI — 94,92% < 90%, vermelho em CI e verde localmente.
  `drivers::discover_at` e as variantes `*_at` tornam a árvore de decisão
  hermética (sysroot sintético exercita todos os ramos nos testes); os
  wrappers públicos continuam sondando o hardware real. Bônus de honestidade:
  `rapl_wrap_com_range_zero_nao_inventa_potencia` agora alcança de fato o
  ramo `range == 0` do wrap (antes o Δt < 1 ms do relógio real desviava o
  par para o ramo degenerado). Total determinístico: **95,20%** em qualquer
  host (linhas; `drivers.rs` 99,65%).
- Diagramas Mermaid do livro não renderizavam — os blocos ```mermaid
  publicavam como código cru. Duas causas somadas: (1) o mdBook 0.5
  deixou de copiar arquivos estáticos soltos de `src/` e `theme/`, então
  `mermaid.min.js` nunca chegava ao artefato (404 no site); (2) o
  `mermaid-init.js` resolvia a URL da lib com `document.currentScript`
  dentro do `DOMContentLoaded` — sempre `null`, e o caminho saía
  relativo à página. A lib agora entra pelo `additional-js` do
  `book.toml` (copiada com hash e emitida como `<script>` antes do
  init; `site.test.js` passa a validar a fonte em `web/vendor/`), e o
  init captura a própria base no tempo de execução da tag.
