# -*- coding: utf-8 -*-
"""Testes do JS do dashboard (web/badge.js) via node.

Regra de ouro (AGENTS §2): testes primeiro — o comportamento do badge do
modelo (a matriz do 401 "sem chave ≠ modelo inativo") é definida em
tests/unit/web/badge.test.js e executada aqui no pytest, para entrar na
mesma suíte e no CI (`make test-unit` → pytest; `make check` cobre a
sintaxe). Requer node (requisito declarado do Makefile).
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
TESTE = ROOT / "tests" / "unit" / "web" / "badge.test.js"


def test_badge_js_matriz_do_modelo():
    node = shutil.which("node")
    if node is None:
        pytest.skip("node indisponível (requisito do Makefile)")
    r = subprocess.run([node, str(TESTE)], capture_output=True, text=True, timeout=30)
    assert r.returncode == 0, f"node falhou:\n{r.stdout}\n{r.stderr}"
