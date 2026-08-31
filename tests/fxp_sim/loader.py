# -*- coding: utf-8 -*-
"""Carregador do IR no runtime do protótipo — mock do front-end da Etapa 1.

Na Etapa 2 (PLAN §2.1) o lexer/parser produzirá o IR direto do texto `.vl`;
aqui os testes carregam o IR construído por `ir.py`. Toda a semântica de
validação estrutural fica no `contract.py` — carregar um programa inválido
é bug do teste, não do loader.
"""

from __future__ import annotations

from . import ir
from .blueprint import load as _load_prototype


def _vbl():
    return _load_prototype()


def load(engine, program: dict):
    """Carrega formas, reviews e o bloco main no engine.

    Retorna o `MainInterpreter` (ou None se o programa não tiver main).
    """
    for declaration in program["declarations"]:
        if declaration["type"] == "form":
            _load_form(engine, declaration)
        elif declaration["type"] == "review":
            _load_review(engine, declaration)
        else:
            raise ValueError(f"declaração desconhecida: {declaration['type']}")

    block = program.get("main")
    if block is None:
        return None
    return _load_main(engine, block)


# ----------------------------------------------------------------------
def _load_form(engine, decl: dict):
    attributes = decl["attributes"]
    value = attributes["value"]
    horizon = ir.duration(attributes["horizon"])
    source_path = attributes.get("source_path")
    now = engine.sim_time
    name = decl["name"]

    if decl["conjugation"] == "event":
        form_obj = _vbl().EventForm(name, value, horizon, source_path, now)
    elif decl["conjugation"] == "equilibrium":
        form_obj = _vbl().EquilibriumForm(
            name, value, horizon, source_path, attributes.get("cost_bytes"), now
        )
    elif decl["conjugation"] == "nonequilibrium":
        if "maintenance_deadline" not in attributes:
            raise ValueError(
                f"forma '{name}': nonequilibrium exige maintenance_deadline "
                f"(validação estrutural fica no contract.py)"
            )
        form_obj = _vbl().NonequilibriumForm(
            name, value, horizon, source_path,
            ir.duration(attributes["maintenance_deadline"]),
            attributes.get("exchange_mode", "cooperation"), now,
        )
    else:
        raise ValueError(f"conjugação desconhecida: {decl['conjugation']}")

    # opcionais comuns (sobrepõem o padrão da conjugação)
    if "currency" in attributes:
        form_obj.currency = attributes["currency"]
    if "classification" in attributes:
        form_obj.classification_currency = attributes["classification"]

    engine.register_form(form_obj)
    return form_obj


def _load_review(engine, decl: dict):
    form_obj = engine.forms.get(decl["form"])
    if form_obj is None:
        raise ValueError(f"review para forma inexistente: {decl['form']}")
    for rule in decl["rules"]:
        threshold = rule["threshold"]  # unidade vira número puro (FORMAL §3)
        form_obj.add_review_condition(
            rule["sensor"], rule["op"], threshold,
            [_translate_action(a) for a in rule["actions"]],
        )
    return form_obj


def _translate_action(action: dict) -> dict:
    """IR -> estrutura esperada pelo protótipo (actor/value)."""
    return dict(action)


def _load_main(engine, block: dict):
    """Traduz statements do IR para a estrutura esperada pelo MainInterpreter."""
    interpreter = _vbl().MainInterpreter(engine)
    periodic: list[tuple[float, list[dict]]] = []

    def translate(stmt: dict) -> dict:
        kind = stmt["statement"]
        if kind == "keep":
            return {"statement": "keep", "form": stmt["form"]}
        if kind == "act":
            return {"statement": "act", "actor": stmt["actor"], "value": stmt["value"]}
        if kind == "every":
            periodic.append(
                (ir.duration(stmt["period"]), [translate(s) for s in stmt["statements"]])
            )
            return {}
        raise ValueError(f"statement desconhecido: {kind}")

    direct = []
    for stmt in block["statements"]:
        translated = translate(stmt)
        if translated:
            direct.append(translated)
    if direct:
        # statements de topo do main rodam como um bloco `every` de período 1 tick
        periodic.insert(0, (engine.tick_seconds, direct))
    for period, statements in periodic:
        interpreter.add_every(period, statements)
    return interpreter
