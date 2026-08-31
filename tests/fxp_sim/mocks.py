# -*- coding: utf-8 -*-
"""Fronteira mock em processo do FXP (docs/PLAN.md §1.3 b).

Diferença em relação ao `FXPSimulator` (PLAN §6.5): o mock NÃO tem modelo
físico, cronograma nem política de fallback — é a fronteira mínima de
dicionários para testes unitários focados (serialização de mensagens,
contrato de leitura, entregas). Sem schema binário: o schema v1 do FXP é
entregável da Etapa 3 (PLAN §3.5).
"""

from __future__ import annotations


class MockFXP:
    """FXP falso, em processo, para testes unitários focados."""

    def __init__(self, ledger=None):
        self.ledger = ledger  # classe Caderno do protótipo ou None
        self.sensors: dict[str, float] = {}
        self.actors: dict[str, dict] = {}   # nome -> {min,max,safety,current}
        self.outbox: list[dict] = []        # mensagens serializadas
        self.delivered: list[dict] = []     # comandos aplicados
        self._seq = 0
        self.cpu_power = 0.0
        self.disk_bytes_used = 0

    # -- registro -------------------------------------------------------
    def register_sensor(self, name: str, value: float = 0.0):
        self.sensors[name] = float(value)

    def register_actor(self, name: str, min_value=None, max_value=None, safety=None):
        self.actors[name] = {"min": min_value, "max": max_value,
                             "safety": safety, "current": None}

    # -- interface do runtime -------------------------------------------
    def read_sensor(self, name: str, timeout_s: float = 0.001) -> float | None:
        if name not in self.sensors:
            if self.ledger is not None:
                self.ledger.alert(
                    f"Sensor '{name}' não registrado no FXP (falha de I/O).",
                    motivo="sensor_nao_registrado", sensor=name,
                )
            return None  # nunca 0.0 — zero é leitura válida (FORMAL §4.7)
        return self.sensors[name]

    def act(self, actor_name: str, value) -> bool:
        self._seq += 1
        message = {"seq": self._seq, "op": "act", "actor": actor_name,
                   "value": value}
        self.outbox.append(message)
        actor = self.actors.get(actor_name)
        if actor is None:
            if self.ledger is not None:
                self.ledger.event("ator_inexistente",
                                  f"Ator '{actor_name}' não registrado no FXP.",
                                  ator=actor_name)
            return False
        if actor["min"] is not None and value < actor["min"]:
            if self.ledger is not None:
                self.ledger.event("actor_rejected_value",
                                  f"Comando a '{actor_name}' rejeitado (min).",
                                  ator=actor_name, valor=value,
                                  limite="min", limite_valor=actor["min"])
            return False
        if actor["max"] is not None and value > actor["max"]:
            if self.ledger is not None:
                self.ledger.event("actor_rejected_value",
                                  f"Comando a '{actor_name}' rejeitado (max).",
                                  ator=actor_name, valor=value,
                                  limite="max", limite_valor=actor["max"])
            return False
        if actor["safety"] is not None and value > actor["safety"]:
            if self.ledger is not None:
                self.ledger.event("actor_rejected_value",
                                  f"Comando a '{actor_name}' rejeitado (safety).",
                                  ator=actor_name, valor=value,
                                  limite="safety_limit", limite_valor=actor["safety"])
            return False
        actor["current"] = value
        self.delivered.append({**message, "event": "delivery"})
        return True

    def update_hardware_state(self):
        pass  # mock sem modelo físico
