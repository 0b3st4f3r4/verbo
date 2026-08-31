# -*- coding: utf-8 -*-
"""Utilidades de asserção sobre o Caderno (docs/FORMAL.md §6).

O Caderno do protótipo é uma classe com estado de classe (`_events`); os
testes precisam isolar cenários (`Caderno.reset()`) e consultar eventos por
`kind` e campos extras. `ConsultaCaderno` encapsula essas consultas sem
poluir o protótipo com API de teste.
"""

from __future__ import annotations


class ConsultaCaderno:
    """Visão consultável dos eventos do Caderno."""

    def __init__(self, caderno_cls):
        self._cls = caderno_cls

    @property
    def eventos(self) -> list[dict]:
        return list(self._cls._events)

    def event(self, kind: str, msg: str, **extra) -> dict:
        """Pass-through para semear eventos nos testes."""
        return self._cls.event(kind, msg, **extra)

    def reset(self):
        self._cls.reset()

    def kinds(self) -> list[str]:
        return [e["kind"] for e in self._cls._events]

    def buscar(self, kind: str | None = None, **extras) -> list[dict]:
        """Eventos que casam kind (exato) e todos os extras informados."""
        achados = []
        for e in self._cls._events:
            if kind is not None and e["kind"] != kind:
                continue
            if any(e.get(chave) != valor for chave, valor in extras.items()):
                continue
            achados.append(e)
        return achados

    def tem(self, kind: str | None = None, **extras) -> bool:
        return bool(self.buscar(kind, **extras))

    def contem_msg(self, trecho: str) -> bool:
        return any(trecho in e["msg"] for e in self._cls._events)

    def chain_head(self) -> str:
        return self._cls.chain_head()

    def verify_chain(self) -> bool:
        return self._cls.verify_chain()
