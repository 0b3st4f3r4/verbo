# -*- coding: utf-8 -*-
"""IR (representação intermediária) dos programas VerboLang para os testes.

É a fronteira do compilador na Etapa 1: os testes constroem programas nesta
estrutura e o `loader` carrega no runtime. Quando o parser real existir
(Etapa 2 — PLAN §2.1), ele produzirá esta mesma estrutura — os testes não
mudam de shape, apenas de origem.

Formas:
  {"tipo": "forma", "conjugacao": "nonequilibrium", "nome": "PensarLivre",
   "atributos": {"value": ..., "horizon": "60s", ...}}   # ordem preservada
Reviews:
  {"tipo": "review", "forma": "PensarLivre", "regras": [...]}
  regra = {"sensor": str, "op": str, "limiar": float, "unidade": str|None,
           "acoes": [{"action": ...}, ...]}
Main:
  {"tipo": "main", "statements": [...]}
Programa:
  {"declaracoes": [...], "main": bloco|None}
"""

from __future__ import annotations

import re

# duração "NUM[unit]" — decimais válidas (FORMAL §3): 2.5s, 500ms, 3s ...
_DURACAO = re.compile(r"^(?P<num>\d+(?:\.\d+)?)(?P<unit>s|ms|us|ns)$")

CONJUGACOES = ("event", "equilibrium", "nonequilibrium")
OPERADORES = ("<", ">", "<=", ">=", "==", "!=")
ACOES = ("dissolve", "subvert", "reclassify_as_equilibrium",
         "reclassify_as_nonequilibrium", "notify_shutdown", "act")
UNIDADE_POR_GRANDEZA = {
    "temperatura": "°C",
    "potencia": "W",
    "atencao": "%",
}


def duracao(texto: str) -> float:
    """Converte '3s'/'2.5s'/'500ms'/'200us'/'100ns' para segundos (float)."""
    m = _DURACAO.match(texto) if isinstance(texto, str) else None
    if m is None:
        raise ValueError(f"duração inválida: {texto!r} (esperado NUM[s|ms|us|ns])")
    fator = {"s": 1.0, "ms": 1e-3, "us": 1e-6, "ns": 1e-9}[m.group("unit")]
    return float(m.group("num")) * fator


# ----------------------------------------------------------------------
# Builders
# ----------------------------------------------------------------------
def forma(nome: str, conjugacao: str, valor="conteudo", horizon: str = "3s",
          **opcionais) -> dict:
    """Forma canônica: `value` primeiro, `horizon` depois (FORMAL §3)."""
    atributos: dict = {"value": valor, "horizon": horizon}
    atributos.update(opcionais)
    return {"tipo": "forma", "conjugacao": conjugacao, "nome": nome,
            "atributos": atributos}


def acao(nome: str, **args) -> dict:
    if nome not in ACOES:
        raise ValueError(f"ação desconhecida: {nome}")
    return {"action": nome, **args}


def act_(ator: str, valor) -> dict:
    return acao("act", ator=ator, valor=valor)


def regra(sensor: str, op: str, limiar, unidade: str | None = None,
          *acoes) -> dict:
    if op not in OPERADORES:
        raise ValueError(f"operador inválido: {op}")
    return {"sensor": sensor, "op": op, "limiar": float(limiar),
            "unidade": unidade, "acoes": list(acoes)}


def review(nome_forma: str, *regras) -> dict:
    return {"tipo": "review", "forma": nome_forma, "regras": list(regras)}


# -- statements do bloco main (FORMAL §3) --------------------------------
def keep_(forma_nome: str) -> dict:
    return {"statement": "keep", "forma": forma_nome}


def act_main(ator: str, valor) -> dict:
    return {"statement": "act", "ator": ator, "valor": valor}


def every(periodo: str, *statements) -> dict:
    return {"statement": "every", "periodo": periodo, "statements": list(statements)}


def main_bloco(*statements) -> dict:
    return {"tipo": "main", "statements": list(statements)}


def programa(*declaracoes, main: dict | None = None) -> dict:
    return {"declaracoes": list(declaracoes), "main": main}
