# -*- coding: utf-8 -*-
"""Utilidades de asserção sobre o Caderno (docs/FORMAL.md §6).

O Caderno do protótipo é uma classe com estado de classe (`_events`); os
testes precisam isolar cenários (`Caderno.reset()`) e consultar eventos por
`kind` e campos extras. `LedgerQuery` encapsula essas consultas sem
poluir o protótipo com API de teste.
"""

from __future__ import annotations


class LedgerQuery:
    """Visão consultável dos eventos do Caderno."""

    def __init__(self, ledger_cls):
        self._cls = ledger_cls

    @property
    def events(self) -> list[dict]:
        return list(self._cls._events)

    def event(self, kind: str, msg: str, **extra) -> dict:
        """Pass-through para semear eventos nos testes."""
        return self._cls.event(kind, msg, **extra)

    def reset(self):
        self._cls.reset()

    def kinds(self) -> list[str]:
        return [e["kind"] for e in self._cls._events]

    def find(self, kind: str | None = None, **extras) -> list[dict]:
        """Eventos que casam kind (exato) e todos os extras informados."""
        found = []
        for e in self._cls._events:
            if kind is not None and e["kind"] != kind:
                continue
            if any(e.get(key) != value for key, value in extras.items()):
                continue
            found.append(e)
        return found

    def has(self, kind: str | None = None, **extras) -> bool:
        return bool(self.find(kind, **extras))

    def contains_message(self, fragment: str) -> bool:
        return any(fragment in e["msg"] for e in self._cls._events)

    def chain_head(self) -> str:
        return self._cls.chain_head()

    def verify_chain(self) -> bool:
        return self._cls.verify_chain()
