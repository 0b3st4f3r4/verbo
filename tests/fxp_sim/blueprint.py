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

ROOT = pathlib.Path(__file__).resolve().parents[2]
PROTOTYPE = ROOT / "prototype" / "verbolang-complete-blueprint.py"
_MODULE_NAME = "verbolang_blueprint"


def load():
    """Importa o blueprint do protótipo (uma única vez por processo)."""
    if _MODULE_NAME in sys.modules:
        return sys.modules[_MODULE_NAME]
    spec = importlib.util.spec_from_file_location(_MODULE_NAME, PROTOTYPE)
    if spec is None or spec.loader is None:  # pragma: no cover - defesa
        raise ImportError(f"não foi possível carregar {PROTOTYPE}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[_MODULE_NAME] = module
    spec.loader.exec_module(module)
    return module
