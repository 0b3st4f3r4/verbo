# Makefile — atalhos do projeto VerboLang
# Requisitos: GNU make, bash, node, curl (e a GPU para `serve`/`up`).
# `make help` lista tudo. `.serve.log` já coberto pelo .gitignore (*.log).

.RECIPEPREFIX := >
SHELL := /bin/bash

SCRIPT  := scripts/serve-local-llm.sh
WEBUI   := scripts/webui.py
UI_PORT ?= 8188
UI_BASE := http://127.0.0.1:$(UI_PORT)/web/chat.html
# chave na mesma ordem do script: env → credenciais do DSH
KEY := $(shell awk '/^LOCAL_VLLM_KEY:/{print $$2}' $${LOCAL_VLLM_KEY_FILE:-$(HOME)/.dsh}/.credentials.yaml 2>/dev/null)
LOG := .serve.log

# Suíte de testes (Etapa 1): usa o venv do workspace se existir
PYTHON ?= python3
PYTHON_BIN := $(if $(wildcard .venv/bin/python),.venv/bin/python,$(PYTHON))

# Núcleo Rust (Etapa 2): CARGO_HOME dentro do workspace (HOME é read-only)
CARGO ?= cargo
CARGO_HOME := $(abspath core/.cargo-home)
export CARGO_HOME
NIGHTLY ?= nightly

.PHONY: help check smoke release-check serve up stop ui web setup test test-unit test-bdd validate-cheatsheet hooks site-check site-build site
.PHONY: rust-build rust-test rust-e2e rust-lint rust-asan rust-bench rust-check rust-package rust-coverage rust-clean
.PHONY: rust-memoria rust-soak

help:
> @echo "VerboLang — atalhos:"
> @echo "  make check   valida shell + JS inline da UI (rápido, offline)"
> @echo "  make smoke   testa endpoints locais (precisa do servidor no ar)"
> @echo "  make release-check  bateria antes da tag: check + testes web + site-check + smoke (docs/RELEASES.md)"
> @echo "  make site-check  valida o livro do site (SUMMARY, links, assets — sem compilar)"
> @echo "  make site-build  monta site/src e compila o livro (mdbook ≥ 0.5)"
> @echo "  make site        compila e serve o livro em http://127.0.0.1:$(SITE_PORT)/"
> @echo "  make setup   cria .venv e instala requirements-dev.txt"
> @echo "  make hooks   ativa o pre-commit (.githooks → core.hooksPath): o CI inteiro antes de cada commit"
> @echo "  make test    suíte completa: unitários (pytest) + BDD (behave)"
> @echo "  make test-unit  apenas testes unitários (pytest)"
> @echo "  make test-bdd   apenas cenários BDD (behave)"
> @echo "  --- núcleo Rust (Etapas 2–4) ---"
> @echo "  make rust-check    parser/runtime/FXP/Caderno: clippy + todos os testes"
> @echo "  make rust-build    compila o workspace core/ (vbl, vbl-lang, vbl-runtime, vbl-fxp)"
> @echo "  make rust-test     testes: matriz (42), canon (5), transição (36), FXP (42), Caderno (12)"
> @echo "  make rust-e2e      E2E da Etapa 4: CLI + FXP + Caderno de produção (7 cenários)"
> @echo "  make rust-lint     clippy --workspace --all-targets (zero warnings)"
> @echo "  make rust-package  empacota os 4 crates p/ crates.io (dry-run de publicação)"
> @echo "  make rust-asan     testes sob AddressSanitizer (vazamentos, AGENTS §1.3)"
> @echo "  make rust-bench    criterion: transição ≤100µs p95, escalonador, FXP, Caderno (gravação ≤200µs)"
> @echo "  make rust-coverage cobertura via cargo-llvm-cov (relatório em core/target)"
> @echo "  make rust-memoria orçamentos de heap (Etapa 5; auditor serial: --test-threads=1)"
> @echo "  make rust-soak     execução longa com churn (Etapa 5; VIVAS/TICKS/SEGUNDOS)"
> @echo "  make rust-clean    limpa core/target"
> @echo "  make validate-cheatsheet  banco de 20 prompts contra o LLM local (PLAN §7)"
> @echo "  make serve   sobe a UI/dashboard (na hora, sem depender do modelo) e o modelo em primeiro plano"
> @echo "  make up      sobe em segundo plano (log em $(LOG))"
> @echo "  make web     só a UI/dashboard + métricas do Caderno (sem GPU; porta $(UI_PORT))"
> @echo "  make stop    encerra vLLM e o servidor da UI"
> @echo "  make ui      imprime a URL da UI (com a chave, se encontrada)"

check:
> @set -e; \
  bash -n $(SCRIPT) && echo "✓ shell"; \
  python3 -m py_compile $(WEBUI) && echo "✓ ponte de métricas"; \
  node --check web/badge.js && echo "✓ badge.js"; \
  sed -n '/^<script>$$/,/^<\/script>$$/p' web/chat.html | sed '1d;$$d' > .ui-check.js; \
  node --check .ui-check.js && rm -f .ui-check.js && echo "✓ js da UI"; \
  for f in web/index.html web/metrics.html; do \
    sed -n '/^<script>$$/,/^<\/script>$$/p' $$f | sed '1d;$$d' > .ui-check.js; \
    node --check .ui-check.js && echo "✓ js de $$f"; \
  done; \
  rm -f .ui-check.js

# Bateria obrigatória antes de marcar qualquer tag (docs/RELEASES.md § Cortando uma release).
release-check:
> @set -e; \
  echo "── release-check: bateria da release ──"; \
  $(MAKE) --no-print-directory check; \
  node --test tests/unit/web/*.test.js; \
  $(MAKE) --no-print-directory site-check; \
  $(MAKE) --no-print-directory smoke; \
  echo "✓ release-check OK — atualize o CHANGELOG.md e marque a tag anotada"

smoke:
> @set -e; \
  for p in web/index.html web/chat.html web/metrics.html web/docs.html web/md.js docs/cheatsheet/VBL-CHEATSHEET.md \
           docs/cheatsheet/VBL-CHEATSHEET-AGENTES.md web/verbolog.svg \
           web/vendor/mermaid.min.js web/vendor/katex/katex.min.js; do \
    code=$$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$(UI_PORT)/$$p"); \
    echo "$$code  $$p"; \
    test "$$code" = 200; \
  done; \
  code=$$(curl -s -o /dev/null -w '%{http_code}' \
    "http://127.0.0.1:$(UI_PORT)/api/snapshot?src=logs/stage4/thermal-subversion.vcad"); \
  echo "$$code  /api/snapshot (métricas)"; \
  test "$$code" = 200; \
  code=$$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $(KEY)" \
    http://127.0.0.1:8000/v1/models); \
  echo "$$code  /v1/models"; \
  test "$$code" = 200

setup:
> @$(PYTHON) -m venv .venv
> @.venv/bin/pip install -r requirements-dev.txt
> @echo "venv pronto (.venv) — use make test"

# Pre-commit (.githooks/pre-commit): espelho do CI + gates de cobertura ≥95%
# (pytest-cov e cargo-llvm-cov). Modo rápido por commit: VBL_PRE_COMMIT=quick.
hooks:
> @git config core.hooksPath .githooks
> @echo "pre-commit ativado — teste com: bash .githooks/pre-commit --quick"

# ── site de documentação (site/, mdBook — verbolang.org/docs) ─────────────
# mdbook ≥ 0.5 (testado em 0.5.4): cargo install mdbook --locked
#   make site-check  portaria hermética: montagem + validação (sem compilar)
#   make site-build  monta src/ e compila o livro em site/book/
#   make site        build + servidor local em http://127.0.0.1:$(SITE_PORT)/
MDBOOK ?= mdbook
SITE_PORT ?= 8288

site-check:
> @python3 scripts/build_site.py --check
> @node --check site/theme/i18n.js && node --check site/theme/verbolang.js && node --check site/theme/mermaid-init.js
> @node --test tests/unit/web/site.test.js

site-build: site-check
> @python3 scripts/build_site.py
> @if command -v $(MDBOOK) >/dev/null 2>&1; then \
>   $(MDBOOK) build site; \
> else \
>   echo "mdbook não encontrado — instale com: cargo install mdbook --locked (≥ 0.5)"; exit 1; \
> fi

site: site-build
> @echo "livro em site/book — servindo em http://127.0.0.1:$(SITE_PORT)/ (Ctrl+C encerra)"
> @python3 -m http.server $(SITE_PORT) --directory site/book

test:
> @$(PYTHON_BIN) -m pytest -q tests/unit
> @$(PYTHON_BIN) -m behave tests/features

test-unit:
> @$(PYTHON_BIN) -m pytest -q tests/unit

test-bdd:
> @$(PYTHON_BIN) -m behave tests/features

# ------------------------------------------------------------------
# Núcleo Rust — Etapa 2 (core/: vbl-lang, vbl-runtime, vbl-cli)
# ------------------------------------------------------------------
rust-build:
> @cd core && $(CARGO) build

rust-test:
> @cd core && $(CARGO) test

# E2E da Etapa 4 (PLAN §4.2): interpretador integrado + FXP + Caderno de
# produção, com verificação externa dos logs (vbl ledger-verify)
rust-e2e:
> @cd core && $(CARGO) test -p vbl-cli --test e2e

rust-lint:
> @cd core && $(CARGO) clippy --workspace --all-targets -- -D warnings

rust-asan:
> @cd core && RUSTFLAGS="-Zsanitizer=address" $(CARGO) +$(NIGHTLY) test \
>   --workspace --target x86_64-unknown-linux-gnu

rust-check: rust-lint rust-test

# Publicação no crates.io (RELEASES.md § crates.io): empacota o workspace e
# roda o verify build de cada pacote sem tocar no registry. Local permite
# árvore suja (checa antes do commit); o CI roda estrito, sem --allow-dirty.
rust-package:
> @cd core && $(CARGO) package --workspace --locked --allow-dirty

rust-bench:
> @cd core && $(CARGO) bench --bench transition --bench scheduler --bench fxp --bench ledger

rust-coverage:
> @cd core && $(CARGO) +$(NIGHTLY) llvm-cov --workspace --html --output-dir target/coverage

# Etapa 5 (PLAN §5.1): fechamento físico dos orçamentos de heap. O auditor
# mede o PROCESSO — os testes de memória exigem execução serial.
rust-memoria:
> @cd core && $(CARGO) test -p vbl-runtime --features heap-audit --test memoria -- --test-threads=1 --nocapture

# Etapa 5 (AGENTS §2.2: zero vazamentos em longa execução). Padrão: 24 h de
# parede. Sessão/CI: `make rust-soak SEGUNDOS=60` ou `TICKS=5000`.
rust-soak:
> @cd core && $(CARGO) build --release --bin vbl-soak
> @core/target/release/vbl-soak --alive-forms $${VIVAS:-1000} --ticks $${TICKS:-100000000} --seconds $${SEGUNDOS:-86400} --report $${RELATORIO:-100000}

rust-clean:
> @cd core && $(CARGO) clean

validate-cheatsheet:
> @$(PYTHON_BIN) scripts/validate_cheatsheet.py --base-url $${VBL_LLM_URL:-http://127.0.0.1:8000/v1} --model $${VBL_LLM_MODEL:-qwen3-4b}

serve:
> @bash $(SCRIPT)

# Dashboard sem GPU: UI + métricas do Caderno, sem o modelo local
web:
> @python3 $(WEBUI) $(UI_PORT)

up:
> @nohup bash $(SCRIPT) > $(LOG) 2>&1 & echo "PID $$! — log: $(LOG) (make stop para encerrar)"

stop:
> @pkill -f '[s]erve-local-llm' 2>/dev/null || true; \
  pkill -f '[v]llm serve' 2>/dev/null || true; \
  pkill -f '[w]ebui\.py $(UI_PORT)' 2>/dev/null || true; \
  pkill -f '[h]ttp\.server $(UI_PORT)' 2>/dev/null || true; \
  echo "encerrado (se estava no ar)"

ui:
> @if [ -n "$(KEY)" ]; then \
    echo "$(UI_BASE)#k=$(KEY)&u=http%3A%2F%2F127.0.0.1%3A8000%2Fv1&m=qwen3-4b&c=4096"; \
  else \
    echo "$(UI_BASE)   (sem chave; suba com make serve/up que a URL completa é aberta)"; \
  fi
