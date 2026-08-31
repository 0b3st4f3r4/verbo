# Changelog

Todas as mudanças notáveis do VerboLang ficam registradas neste arquivo. O
formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e o
processo/calendário de releases está em
[`docs/RELEASES.md`](docs/RELEASES.md): `vYYYY.N` — 6 meses de P&D + 6 anos
de suporte + 6 meses de descontinuação = 7 anos de ciclo de vida.

## [Não lançado] — linha `v2027.0`, fase alpha (pesquisa, experimentação e definição de escopo)

### Adicionado
- Publicação no crates.io (RELEASES.md § crates.io): workflow
  `publish.yml` (tag `v*` + disparo manual com dry-run, ordem de dependência
  com espera de indexação, idempotente), metadados de pacote nos quatro
  crates (`repository`, `rust-version`, `readme`, `keywords`,
  `categories`), dependências internas com `version` + `path` no workspace
  e alvo `make rust-package` + passo de empacotamento no CI de push.
  Estreia pública em `0.1.0-alpha.0` — análogo SemVer da gramática
  `-alphaN` (as versões internas `0.x` anteriores nunca foram lançadas).
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
