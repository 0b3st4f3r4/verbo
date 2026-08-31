# -*- coding: utf-8 -*-
"""Ambiente do behave — um mundo novo, determinístico, por cenário."""

from __future__ import annotations

import os
import shutil
import sys
import tempfile
from pathlib import Path

RAIZ = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(RAIZ / "tests"))

from fxp_sim import blueprint  # noqa: E402
from fxp_sim import ir, loader  # noqa: E402
from fxp_sim.simulator import FXPSimulator  # noqa: E402
from fxp_sim.support import ConsultaCaderno  # noqa: E402


def before_all(context):
    context.vbl = blueprint.carregar()
    context.ir = ir
    context.loader = loader


def before_scenario(context, scenario):
    context.vbl.Caderno.reset()
    context.tmp = tempfile.mkdtemp(prefix="vbl-etapa1-")
    context.sim = FXPSimulator(caderno=context.vbl.Caderno)
    context.engine = context.vbl.VerboLangEngine(
        fxp=context.sim, persistence_dir=os.path.join(context.tmp, "persistencia")
    )
    context.cad = ConsultaCaderno(context.vbl.Caderno)


def after_scenario(context, scenario):
    shutil.rmtree(context.tmp, ignore_errors=True)
