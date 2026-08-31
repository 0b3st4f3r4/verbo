# -*- coding: utf-8 -*-
"""IR (representação intermediária) dos programas VerboLang para os testes.

É a fronteira do compilador na Etapa 1: os testes constroem programas nesta
estrutura e o `loader` carrega no runtime. Quando o parser real existir
(Etapa 2 — PLAN §2.1), ele produzirá esta mesma estrutura — os testes não
mudam de shape, apenas de origem.

Formas:
  {"type": "form", "conjugation": "nonequilibrium", "name": "FreeThinking",
   "attributes": {"value": ..., "horizon": "60s", ...}}   # ordem preservada
Reviews:
  {"type": "review", "form": "FreeThinking", "rules": [...]}
  rule = {"sensor": str, "op": str, "threshold": float, "unit": str|None,
          "actions": [{"action": ...}, ...]}
Main:
  {"type": "main", "statements": [...]}
Programa:
  {"declarations": [...], "main": bloco|None}
"""

from __future__ import annotations

import re

# duração "NUM[unit]" — decimais válidas (FORMAL §3): 2.5s, 500ms, 3s ...
_DURATION = re.compile(r"^(?P<num>\d+(?:\.\d+)?)(?P<unit>s|ms|us|ns)$")

CONJUGATIONS = ("event", "equilibrium", "nonequilibrium")
OPERATORS = ("<", ">", "<=", ">=", "==", "!=")
ACTIONS = ("dissolve", "subvert", "reclassify_as_equilibrium",
           "reclassify_as_nonequilibrium", "notify_shutdown", "act")
UNIT_BY_QUANTITY = {
    "temperature": "°C",
    "power": "W",
    "attention": "%",
}


def duration(text: str) -> float:
    """Converte '3s'/'2.5s'/'500ms'/'200us'/'100ns' para segundos (float)."""
    m = _DURATION.match(text) if isinstance(text, str) else None
    if m is None:
        raise ValueError(f"duração inválida: {text!r} (esperado NUM[s|ms|us|ns])")
    factor = {"s": 1.0, "ms": 1e-3, "us": 1e-6, "ns": 1e-9}[m.group("unit")]
    return float(m.group("num")) * factor


# ----------------------------------------------------------------------
# Builders
# ----------------------------------------------------------------------
def form(name: str, conjugation: str, value="conteudo", horizon: str = "3s",
         **optional) -> dict:
    """Forma canônica: `value` primeiro, `horizon` depois (FORMAL §3)."""
    attributes: dict = {"value": value, "horizon": horizon}
    attributes.update(optional)
    return {"type": "form", "conjugation": conjugation, "name": name,
            "attributes": attributes}


def action(name: str, **args) -> dict:
    if name not in ACTIONS:
        raise ValueError(f"ação desconhecida: {name}")
    return {"action": name, **args}


def act_(actor: str, value) -> dict:
    return action("act", actor=actor, value=value)


def rule(sensor: str, op: str, threshold, unit: str | None = None,
         *actions) -> dict:
    if op not in OPERATORS:
        raise ValueError(f"operador inválido: {op}")
    return {"sensor": sensor, "op": op, "threshold": float(threshold),
            "unit": unit, "actions": list(actions)}


def review(form_name: str, *rules) -> dict:
    return {"type": "review", "form": form_name, "rules": list(rules)}


# -- statements do bloco main (FORMAL §3) --------------------------------
def keep_(form_name: str) -> dict:
    return {"statement": "keep", "form": form_name}


def act_main(actor: str, value) -> dict:
    return {"statement": "act", "actor": actor, "value": value}


def every(period: str, *statements) -> dict:
    return {"statement": "every", "period": period, "statements": list(statements)}


def main_block(*statements) -> dict:
    return {"type": "main", "statements": list(statements)}


def program(*declarations, main: dict | None = None) -> dict:
    return {"declarations": list(declarations), "main": main}
