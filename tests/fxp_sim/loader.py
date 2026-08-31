# -*- coding: utf-8 -*-
"""Carregador do IR no runtime do protótipo — mock do front-end da Etapa 1.

Na Etapa 2 (PLAN §2.1) o lexer/parser produzirá o IR direto do texto `.vl`;
aqui os testes carregam o IR construído por `ir.py`. Toda a semântica de
validação estrutural fica no `contract.py` — carregar um programa inválido
é bug do teste, não do loader.
"""

from __future__ import annotations

from . import ir
from .blueprint import carregar as _carregar_prototipo


def _vbl():
    return _carregar_prototipo()


def carregar(engine, programa: dict):
    """Carrega formas, reviews e o bloco main no engine.

    Retorna o `MainInterpreter` (ou None se o programa não tiver main).
    """
    for declaracao in programa["declaracoes"]:
        if declaracao["tipo"] == "forma":
            _carregar_forma(engine, declaracao)
        elif declaracao["tipo"] == "review":
            _carregar_review(engine, declaracao)
        else:
            raise ValueError(f"declaração desconhecida: {declaracao['tipo']}")

    bloco = programa.get("main")
    if bloco is None:
        return None
    return _carregar_main(engine, bloco)


# ----------------------------------------------------------------------
def _carregar_forma(engine, decl: dict):
    atributos = decl["atributos"]
    valor = atributos["value"]
    horizon = ir.duracao(atributos["horizon"])
    source_path = atributos.get("source_path")
    agora = engine.sim_time
    nome = decl["nome"]

    if decl["conjugacao"] == "event":
        forma_obj = _vbl().EventForm(nome, valor, horizon, source_path, agora)
    elif decl["conjugacao"] == "equilibrium":
        forma_obj = _vbl().EquilibriumForm(
            nome, valor, horizon, source_path, atributos.get("cost_bytes"), agora
        )
    elif decl["conjugacao"] == "nonequilibrium":
        if "maintenance_deadline" not in atributos:
            raise ValueError(
                f"forma '{nome}': nonequilibrium exige maintenance_deadline "
                f"(validação estrutural fica no contract.py)"
            )
        forma_obj = _vbl().NonequilibriumForm(
            nome, valor, horizon, source_path,
            ir.duracao(atributos["maintenance_deadline"]),
            atributos.get("exchange_mode", "cooperation"), agora,
        )
    else:
        raise ValueError(f"conjugação desconhecida: {decl['conjugacao']}")

    # opcionais comuns (sobrepõem o padrão da conjugação)
    if "currency" in atributos:
        forma_obj.currency = atributos["currency"]
    if "classification" in atributos:
        forma_obj.classification_currency = atributos["classification"]

    engine.register_form(forma_obj)
    return forma_obj


def _carregar_review(engine, decl: dict):
    forma_obj = engine.forms.get(decl["forma"])
    if forma_obj is None:
        raise ValueError(f"review para forma inexistente: {decl['forma']}")
    for regra in decl["regras"]:
        threshold = regra["limiar"]  # unidade vira número puro (FORMAL §3)
        forma_obj.add_review_condition(
            regra["sensor"], regra["op"], threshold,
            [_traduzir_acao(a) for a in regra["acoes"]],
        )
    return forma_obj


def _traduzir_acao(acao: dict) -> dict:
    """IR em português -> estrutura esperada pelo protótipo (actor/value)."""
    traduzida = dict(acao)
    if traduzida["action"] == "act":
        traduzida["actor"] = traduzida.pop("ator")
        traduzida["value"] = traduzida.pop("valor")
    return traduzida


def _carregar_main(engine, bloco: dict):
    """Traduz statements do IR para a estrutura esperada pelo MainInterpreter."""
    interpretador = _vbl().MainInterpreter(engine)
    periodicos: list[tuple[float, list[dict]]] = []

    def traduzir(stmt: dict) -> dict:
        tipo = stmt["statement"]
        if tipo == "keep":
            return {"statement": "keep", "form": stmt["forma"]}
        if tipo == "act":
            return {"statement": "act", "actor": stmt["ator"], "value": stmt["valor"]}
        if tipo == "every":
            periodicos.append(
                (ir.duracao(stmt["periodo"]), [traduzir(s) for s in stmt["statements"]])
            )
            return {}
        raise ValueError(f"statement desconhecido: {tipo}")

    diretos = []
    for stmt in bloco["statements"]:
        traduzido = traduzir(stmt)
        if traduzido:
            diretos.append(traduzido)
    if diretos:
        # statements de topo do main rodam como um bloco `every` de período 1 tick
        periodicos.insert(0, (engine.tick_seconds, diretos))
    for periodo, statements in periodicos:
        interpretador.add_every(periodo, statements)
    return interpretador
