# Changelog

Todas as mudanças notáveis do VerboLang ficam registradas neste arquivo. O
formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e o
processo/calendário de releases está em
[`docs/RELEASES.md`](docs/RELEASES.md): `vYYYY.N` — 6 meses de P&D + 6 anos
de suporte + 6 meses de descontinuação = 7 anos de ciclo de vida.

## [Não lançado]

_(nada ainda — a próxima janela é a `v2027.0.0-alpha.1`, Setembro)_

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
  ordem do CI, com os gates de cobertura ≥ 95% (pytest-cov e llvm-cov).
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
- Gate de cobertura Rust (llvm-cov ≥ 95%) era **dependente do host**: a
  auto-descoberta do FXP (`drivers::discover*`) sondava o `/sys` real, e a
  cobertura variava entre a máquina de referência (AMD, com k10temp/RAPL
  reais) e a VM do CI — 94,92% < 95%, vermelho em CI e verde localmente.
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
