# Makefile — atalhos do projeto VerboLang
# Requisitos: GNU make, bash, node, curl (e a GPU para `serve`/`up`).
# `make help` lista tudo. `.serve.log` já coberto pelo .gitignore (*.log).

.RECIPEPREFIX := >
SHELL := /bin/bash

SCRIPT  := scripts/serve-local-llm.sh
UI_PORT ?= 8188
UI_BASE := http://127.0.0.1:$(UI_PORT)/scripts/verbo-chat/chat.html
# chave na mesma ordem do script: env → credenciais do DSH
KEY := $(shell awk '/^LOCAL_VLLM_KEY:/{print $$2}' $${LOCAL_VLLM_KEY_FILE:-$(HOME)/.dsh}/.credentials.yaml 2>/dev/null)
LOG := .serve.log

# Suíte de testes (Etapa 1): usa o venv do workspace se existir
PYTHON ?= python3
PYTHON_BIN := $(if $(wildcard .venv/bin/python),.venv/bin/python,$(PYTHON))

# Núcleo Rust (Etapa 2): CARGO_HOME dentro do workspace (HOME é read-only)
CARGO ?= cargo
CARGO_HOME := $(abspath nucleo/.cargo-home)
export CARGO_HOME
NIGHTLY ?= nightly

.PHONY: help check smoke serve up stop ui setup test test-unit test-bdd validate-cheatsheet
.PHONY: rust-build rust-test rust-lint rust-asan rust-bench rust-check rust-coverage rust-clean

help:
> @echo "VerboLang — atalhos:"
> @echo "  make check   valida shell + JS inline da UI (rápido, offline)"
> @echo "  make smoke   testa endpoints locais (precisa do servidor no ar)"
> @echo "  make setup   cria .venv e instala requirements-dev.txt"
> @echo "  make test    suíte completa: unitários (pytest) + BDD (behave)"
> @echo "  make test-unit  apenas testes unitários (pytest)"
> @echo "  make test-bdd   apenas cenários BDD (behave)"
> @echo "  --- núcleo Rust (Etapas 2–3) ---"
> @echo "  make rust-check    parser/runtime/FXP: clippy + todos os testes"
> @echo "  make rust-build    compila o workspace nucleo/ (vbl, vbl-lang, vbl-runtime, vbl-fxp)"
> @echo "  make rust-test     testes: matriz (41), canon (5), transição (36), FXP (42)"
> @echo "  make rust-lint     clippy --workspace --all-targets (zero warnings)"
> @echo "  make rust-asan     testes sob AddressSanitizer (vazamentos, AGENTS §1.3)"
> @echo "  make rust-bench    criterion: transição ≤100µs p95, escalonador, FXP (leitura ≤1ms, remota ≤10ms)"
> @echo "  make rust-coverage cobertura via cargo-llvm-cov (relatório em nucleo/target)"
> @echo "  make rust-clean    limpa nucleo/target"
> @echo "  make validate-cheatsheet  banco de 20 prompts contra o LLM local (PLAN §7)"
> @echo "  make serve   sobe o modelo + UI em primeiro plano (Ctrl+C para)"
> @echo "  make up      sobe em segundo plano (log em $(LOG))"
> @echo "  make stop    encerra vLLM e o servidor estático da UI"
> @echo "  make ui      imprime a URL da UI (com a chave, se encontrada)"

check:
> @set -e; \
  bash -n $(SCRIPT) && echo "✓ shell"; \
  sed -n '/^<script>$$/,/^<\/script>$$/p' scripts/verbo-chat/chat.html | sed '1d;$$d' > .ui-check.js; \
  node --check .ui-check.js && rm -f .ui-check.js && echo "✓ js da UI"

smoke:
> @set -e; \
  for p in scripts/verbo-chat/chat.html docs/VBL-CHEATSHEET.md scripts/verbo-chat/verbolog.svg \
           scripts/verbo-chat/vendor/mermaid.min.js scripts/verbo-chat/vendor/katex/katex.min.js; do \
    code=$$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$(UI_PORT)/$$p"); \
    echo "$$code  $$p"; \
    test "$$code" = 200; \
  done; \
  code=$$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $(KEY)" \
    http://127.0.0.1:8000/v1/models); \
  echo "$$code  /v1/models"; \
  test "$$code" = 200

setup:
> @$(PYTHON) -m venv .venv
> @.venv/bin/pip install -r requirements-dev.txt
> @echo "venv pronto (.venv) — use make test"

test:
> @$(PYTHON_BIN) -m pytest -q tests/unit
> @$(PYTHON_BIN) -m behave tests/features

test-unit:
> @$(PYTHON_BIN) -m pytest -q tests/unit

test-bdd:
> @$(PYTHON_BIN) -m behave tests/features

# ------------------------------------------------------------------
# Núcleo Rust — Etapa 2 (nucleo/: vbl-lang, vbl-runtime, vbl-cli)
# ------------------------------------------------------------------
rust-build:
> @cd nucleo && $(CARGO) build

rust-test:
> @cd nucleo && $(CARGO) test

rust-lint:
> @cd nucleo && $(CARGO) clippy --workspace --all-targets -- -D warnings

rust-asan:
> @cd nucleo && RUSTFLAGS="-Zsanitizer=address" $(CARGO) +$(NIGHTLY) test \
>   --workspace --target x86_64-unknown-linux-gnu

rust-check: rust-lint rust-test

rust-bench:
> @cd nucleo && $(CARGO) bench --bench transicao --bench escalonador --bench fxp

rust-coverage:
> @cd nucleo && $(CARGO) +$(NIGHTLY) llvm-cov --workspace --html --output-dir target/coverage

rust-clean:
> @cd nucleo && $(CARGO) clean

validate-cheatsheet:
> @$(PYTHON_BIN) scripts/validate_cheatsheet.py --base-url $${VBL_LLM_URL:-http://127.0.0.1:8000/v1} --model $${VBL_LLM_MODEL:-qwen3-4b}

serve:
> @bash $(SCRIPT)

up:
> @nohup bash $(SCRIPT) > $(LOG) 2>&1 & echo "PID $$! — log: $(LOG) (make stop para encerrar)"

stop:
> @pkill -f '[s]erve-local-llm' 2>/dev/null || true; \
  pkill -f '[v]llm serve' 2>/dev/null || true; \
  pkill -f '[h]ttp\.server $(UI_PORT)' 2>/dev/null || true; \
  echo "encerrado (se estava no ar)"

ui:
> @if [ -n "$(KEY)" ]; then \
    echo "$(UI_BASE)#k=$(KEY)&u=http%3A%2F%2F127.0.0.1%3A8000%2Fv1&m=qwen3-4b&c=4096"; \
  else \
    echo "$(UI_BASE)   (sem chave; suba com make serve/up que a URL completa é aberta)"; \
  fi
