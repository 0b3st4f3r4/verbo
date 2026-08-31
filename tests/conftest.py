# -*- coding: utf-8 -*-
"""Fixtures compartilhadas da suíte unitária da Etapa 1.

Isolamento por teste: `Caderno.reset()` antes de cada caso, simulador FXP
novo e diretório de persistência temporário (tmp_path do pytest).
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

RAIZ = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RAIZ / "tests"))

from fxp_sim import blueprint  # noqa: E402
from fxp_sim.support import ConsultaCaderno  # noqa: E402
from fxp_sim.simulator import FXPSimulator  # noqa: E402


@pytest.fixture(scope="session")
def vbl():
    """Módulo do protótipo de referência (SUT da Etapa 1)."""
    return blueprint.carregar()


@pytest.fixture()
def cad(vbl):
    """Caderno zerado + visão consultável."""
    vbl.Caderno.reset()
    return ConsultaCaderno(vbl.Caderno)


@pytest.fixture()
def sim(vbl):
    """Simulador FXP determinístico (registro mínimo da FORMAL §6)."""
    return FXPSimulator(caderno=vbl.Caderno)


@pytest.fixture()
def engine(vbl, sim, tmp_path):
    """Runtime com FXP injetado e persistência em diretório temporário."""
    return vbl.VerboLangEngine(
        fxp=sim, persistence_dir=str(tmp_path / "persistencia")
    )
