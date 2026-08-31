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

.PHONY: help check smoke serve up stop ui setup test test-unit test-bdd validate-cheatsheet

help:
> @echo "VerboLang — atalhos:"
> @echo "  make check   valida shell + JS inline da UI (rápido, offline)"
> @echo "  make smoke   testa endpoints locais (precisa do servidor no ar)"
> @echo "  make setup   cria .venv e instala requirements-dev.txt"
> @echo "  make test    suíte completa: unitários (pytest) + BDD (behave)"
> @echo "  make test-unit  apenas testes unitários (pytest)"
> @echo "  make test-bdd   apenas cenários BDD (behave)"
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
