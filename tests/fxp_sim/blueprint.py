# -*- coding: utf-8 -*-
"""Carregador do protótipo de referência como SUT da suíte da Etapa 1.

O arquivo `prototype/verbolang-complete-blueprint.py` tem hífens no nome e
não é importável diretamente; aqui ele é carregado via importlib e cacheado
em `sys.modules` para que pytest e behave compartilhem a mesma definição.
"""

from __future__ import annotations

import importlib.util
import pathlib
import sys

RAIZ = pathlib.Path(__file__).resolve().parents[2]
PROTOTIPO = RAIZ / "prototype" / "verbolang-complete-blueprint.py"
_NOME_MODULO = "verbolang_blueprint"


def carregar():
    """Importa o blueprint do protótipo (uma única vez por processo)."""
    if _NOME_MODULO in sys.modules:
        return sys.modules[_NOME_MODULO]
    spec = importlib.util.spec_from_file_location(_NOME_MODULO, PROTOTIPO)
    if spec is None or spec.loader is None:  # pragma: no cover - defesa
        raise ImportError(f"não foi possível carregar {PROTOTIPO}")
    modulo = importlib.util.module_from_spec(spec)
    sys.modules[_NOME_MODULO] = modulo
    spec.loader.exec_module(modulo)
    return modulo
