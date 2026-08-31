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

    def __init__(self, caderno=None):
        self.caderno = caderno  # classe Caderno do protótipo ou None
        self.sensores: dict[str, float] = {}
        self.atores: dict[str, dict] = {}   # nome -> {min,max,safety,atual}
        self.outbox: list[dict] = []        # mensagens serializadas
        self.entregues: list[dict] = []     # comandos aplicados
        self._seq = 0
        self.cpu_power = 0.0
        self.disk_bytes_used = 0

    # -- registro -------------------------------------------------------
    def registrar_sensor(self, nome: str, valor: float = 0.0):
        self.sensores[nome] = float(valor)

    def registrar_ator(self, nome: str, minimo=None, maximo=None, safety=None):
        self.atores[nome] = {"min": minimo, "max": maximo,
                             "safety": safety, "atual": None}

    # -- interface do runtime -------------------------------------------
    def read_sensor(self, name: str, timeout_s: float = 0.001) -> float | None:
        if name not in self.sensores:
            if self.caderno is not None:
                self.caderno.alert(
                    f"Sensor '{name}' não registrado no FXP (falha de I/O).",
                    motivo="sensor_nao_registrado", sensor=name,
                )
            return None  # nunca 0.0 — zero é leitura válida (FORMAL §4.7)
        return self.sensores[name]

    def act(self, actor_name: str, value) -> bool:
        self._seq += 1
        mensagem = {"seq": self._seq, "op": "act", "ator": actor_name,
                    "valor": value}
        self.outbox.append(mensagem)
        ator = self.atores.get(actor_name)
        if ator is None:
            if self.caderno is not None:
                self.caderno.event("ator_inexistente",
                                   f"Ator '{actor_name}' não registrado no FXP.",
                                   ator=actor_name)
            return False
        if ator["min"] is not None and value < ator["min"]:
            if self.caderno is not None:
                self.caderno.event("actor_rejected_value",
                                   f"Comando a '{actor_name}' rejeitado (min).",
                                   ator=actor_name, valor=value,
                                   limite="min", limite_valor=ator["min"])
            return False
        if ator["max"] is not None and value > ator["max"]:
            if self.caderno is not None:
                self.caderno.event("actor_rejected_value",
                                   f"Comando a '{actor_name}' rejeitado (max).",
                                   ator=actor_name, valor=value,
                                   limite="max", limite_valor=ator["max"])
            return False
        if ator["safety"] is not None and value > ator["safety"]:
            if self.caderno is not None:
                self.caderno.event("actor_rejected_value",
                                   f"Comando a '{actor_name}' rejeitado (safety).",
                                   ator=actor_name, valor=value,
                                   limite="safety_limit", limite_valor=ator["safety"])
            return False
        ator["atual"] = value
        self.entregues.append({**mensagem, "evento": "entrega"})
        return True

    def update_hardware_state(self):
        pass  # mock sem modelo físico
