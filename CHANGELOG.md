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
