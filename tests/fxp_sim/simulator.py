# -*- coding: utf-8 -*-
"""Simulador físico determinístico do FXP — esqueleto da Etapa 1 (PLAN §6.5).

Papel na suíte: é o "mundo externo" roteirizado que o runtime consulta a cada
tick. Evolui para um módulo separado do FXP real na Etapa 3 (PLAN §3); aqui
ficam apenas as capacidades exigidas pelos cenários de teste:

- Séries temporais roteirizadas (`schedule`/`set_sensor`) — determinismo total,
  sem aleatoriedade (requisito do PLAN §6.5).
- Injeção de falhas: sensor ausente, sensor registrado porém inacessível,
  ator que não responde (heartbeat), picos térmicos.
- Registro mínimo obrigatório de sensores e atores (FORMAL §6) com limites de
  segurança e validação de comando **inclusiva** (FORMAL §4.3).
- Política de fallback no REGISTRO do FXP (primary → alternativos), conforme
  FORMAL §4.3 — o runtime não implementa fallback próprio (BDD Caso 3).
- Mensagens FXP serializadas em dicionários (fronteira em processo, sem schema
  binário — o schema v1 é entregável da Etapa 3, PLAN §3.5).
- Atores com resposta física plausível e determinística (ex.: ventoinha
  resfria a CPU; power cap limita a potência).

Interface exigida pelo runtime do protótipo (duck typing):
  read_sensor(nome, timeout_s=...) -> float | None
  act(ator, valor) -> bool
  update_hardware_state() -> None
  atributos: cpu_power, disk_bytes_used
"""

from __future__ import annotations

from dataclasses import dataclass, field

# Valor poético canônico (FORMAL §4.5) — usado nas asserções do BDD Caso 2.
CANONICAL_POETIC_VALUE = "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"


@dataclass
class SensorState:
    name: str
    quantity: str          # temperature | power | attention | ...
    unit: str              # °C | W | %
    range: tuple[float, float]
    value: float
    registered: bool = True
    accessible: bool = True


@dataclass
class ActorState:
    name: str
    function: str
    min_value: float | None = None
    max_value: float | None = None
    safety_limit: float | None = None
    current: object = None
    available: bool = True
    fallback: list[str] = field(default_factory=list)
    apply: object = None  # callable(value) -> None | None (efeito simulado)


class FXPSimulator:
    """Barramento FXP simulado, determinístico, para a suíte da Etapa 1."""

    def __init__(self, ledger, tick_seconds: float = 1.0):
        self.ledger = ledger            # classe Caderno do protótipo (injetada)
        self.tick_seconds = tick_seconds
        self.ticks = 0
        self.disk_bytes_used = 1024

        self.values: dict[str, float] = {
            "cpu_temp": 55.0,
            "cpu_power": 150.0,
            "attention": 100.0,
        }
        self.sensors: dict[str, SensorState] = {}
        self._register_default("cpu_temp", "temperature", "°C", (0, 120))
        self._register_default("cpu_power", "power", "W", (0, 500))
        self._register_default("attention", "attention", "%", (0, 100))

        self.actors: dict[str, ActorState] = {}
        self._register_default_actors()

        # Mensagens FXP serializadas (fronteira em processo, sem schema binário)
        self.outbox: list[dict] = []
        self.delivered: list[dict] = []
        self._seq = 0

        # Séries roteirizadas: {tick: {"cpu_temp": 90.0, ...}}
        self.timeline: dict[int, dict[str, float]] = {}

    # ------------------------------------------------------------------
    # Registro (FORMAL §6)
    # ------------------------------------------------------------------
    def _register_default(self, name, quantity, unit, range_):
        self.sensors[name] = SensorState(
            name=name,
            quantity=quantity,
            unit=unit,
            range=range_,
            value=self.values[name],
        )

    def _register_default_actors(self):
        """Registro mínimo obrigatório (FORMAL §6)."""
        self.register_actor(
            "CpuPowerCap", "limite de potência da CPU (W)",
            min_value=10, max_value=250, safety_limit=200,
            apply=lambda v: self._power_cap_effect(v),
        )
        self.register_actor(
            "Ventoinha", "velocidade da ventoinha (PWM)",
            min_value=0, max_value=255, safety_limit=200,
            apply=lambda v: self._fan_effect(v),
        )
        self.register_actor("LedIndicador", "estado textual do LED")

    def register_actor(self, name, function, min_value=None, max_value=None,
                       safety_limit=None, apply=None):
        self.actors[name] = ActorState(
            name=name, function=function, min_value=min_value, max_value=max_value,
            safety_limit=safety_limit, current=None, available=True,
            fallback=[], apply=apply,
        )

    def define_fallback(self, primary: str, *alternatives: str):
        """Política de fallback fica no REGISTRO do FXP (FORMAL §4.3)."""
        self.actors[primary].fallback = list(alternatives)

    def register_sensor(self, name, quantity="generic", unit="",
                        range=(0.0, 1e9), value=0.0):
        self.values[name] = float(value)
        self.sensors[name] = SensorState(name, quantity, unit, range, float(value))

    def unregister_sensor(self, name):
        """Simula sensor não registrado (falha de I/O — FORMAL §4.7)."""
        self.sensors.pop(name, None)

    # ------------------------------------------------------------------
    # Roteirização do mundo e injeção de falhas
    # ------------------------------------------------------------------
    def set_sensor(self, name: str, value: float):
        if name not in self.sensors:
            raise KeyError(f"sensor '{name}' não registrado no simulador")
        self.sensors[name].value = float(value)
        if name in self.values or name in ("cpu_temp", "cpu_power", "attention"):
            self.values[name] = float(value)

    def fail_sensor(self, name: str):
        """Sensor registrado porém inacessível (FORMAL §4.7)."""
        self.sensors[name].accessible = False

    def schedule(self, tick: int, **values: float):
        """Agenda valores absolutos de sensores para um tick (1-based)."""
        self.timeline.setdefault(tick, {}).update(values)

    def fail_actor(self, name: str):
        """Ator para de responder (heartbeat falho — BDD Caso 3)."""
        self.actors[name].available = False

    def recover_actor(self, name: str):
        self.actors[name].available = True

    # ------------------------------------------------------------------
    # Propriedades usadas pelo runtime
    # ------------------------------------------------------------------
    @property
    def cpu_power(self) -> float:
        return self.values["cpu_power"]

    @cpu_power.setter
    def cpu_power(self, value: float):
        self.values["cpu_power"] = float(value)
        if "cpu_power" in self.sensors:
            self.sensors["cpu_power"].value = float(value)

    @property
    def cpu_temperature(self) -> float:
        return self.values["cpu_temp"]

    @cpu_temperature.setter
    def cpu_temperature(self, value: float):
        self.set_sensor("cpu_temp", value)

    @property
    def human_attention(self) -> float:
        return self.values["attention"]

    @human_attention.setter
    def human_attention(self, value: float):
        self.set_sensor("attention", value)

    # ------------------------------------------------------------------
    # Interface consultada pelo runtime
    # ------------------------------------------------------------------
    def read_sensor(self, name: str, timeout_s: float = 0.001) -> float | None:
        """Leitura por nome simbólico. Falha de I/O -> None + alerta (§4.7).

        NUNCA retorna 0.0 para sensor ausente: zero é leitura física válida
        e dispararia falsas condições de revisão (FORMAL §4.7).
        """
        sensor = self.sensors.get(name)
        if sensor is None or not sensor.registered:
            self.ledger.alert(
                f"Sensor '{name}' não registrado no FXP (falha de I/O). "
                f"Condição não avaliada neste tick.",
                motivo="sensor_not_registered", sensor=name,
            )
            return None
        if not sensor.accessible:
            self.ledger.alert(
                f"Sensor '{name}' registrado porém inacessível (falha de leitura). "
                f"Condição não avaliada neste tick.",
                motivo="sensor_inaccessible", sensor=name,
            )
            return None
        return round(float(sensor.value), 2)

    def act(self, actor_name: str, value) -> bool:
        """Comando a ator: serializa, valida limites (inclusivos), entrega.

        FORMAL §4.3: fora dos limites -> rejeitado SEM envio, registrado como
        `actor_rejected_value`; ator indisponível -> política de fallback do
        registro; tentativa, falha e fallback aparecem no Caderno.
        """
        self._seq += 1
        message = {
            "seq": self._seq,
            "op": "act",
            "actor": actor_name,
            "value": value,
            "tick": self.ticks,
        }
        self.outbox.append(message)

        actor = self.actors.get(actor_name)
        if actor is None:
            self.ledger.event(
                "actor_unknown",
                f"Ator '{actor_name}' não registrado no FXP.",
                ator=actor_name,
            )
            self.ledger.actuator_action(actor_name, value, False)
            return False

        if not actor.available:
            self.ledger.actuator_action(actor_name, value, False)
            self.ledger.event(
                "actor_unavailable",
                f"Heartbeat do ator '{actor_name}' não respondeu.",
                ator=actor_name,
            )
            return self._try_fallback(actor, value)

        violation = self._limit_violation(actor, value)
        if violation is not None:
            limit, limit_value = violation
            self.ledger.event(
                "actor_rejected_value",
                f"Comando a '{actor_name}' rejeitado sem envio: valor {value} "
                f"viola {limit} = {limit_value}.",
                ator=actor_name, valor=value,
                limite=limit, limite_valor=limit_value,
            )
            return False

        self._deliver(actor, value, message)
        return True

    # ------------------------------------------------------------------
    # Internos
    # ------------------------------------------------------------------
    @staticmethod
    def _limit_violation(actor: ActorState, value):
        """Limites inclusivos: valor igual ao limite é aceito (FORMAL §4.3)."""
        if actor.min_value is not None and value < actor.min_value:
            return ("min", actor.min_value)
        if actor.max_value is not None and value > actor.max_value:
            return ("max", actor.max_value)
        if actor.safety_limit is not None and value > actor.safety_limit:
            return ("safety_limit", actor.safety_limit)
        return None

    def _deliver(self, actor: ActorState, value, message: dict):
        if actor.apply is not None:
            actor.apply(value)
        actor.current = value
        self.delivered.append({**message, "event": "delivery"})
        self.ledger.actuator_action(actor.name, value, True)

    def _try_fallback(self, primary: ActorState, value) -> bool:
        for alt_name in primary.fallback:
            alternative = self.actors.get(alt_name)
            if alternative is None or not alternative.available:
                continue
            if self._limit_violation(alternative, value) is not None:
                self.ledger.alert(
                    f"Fallback '{alt_name}' rejeitou o valor {value} (limites).",
                    motivo="fallback_rejeitado", ator=alt_name,
                )
                continue
            self._deliver(alternative, value, {
                "seq": self._seq, "op": "act", "actor": alt_name,
                "value": value, "tick": self.ticks, "fallback_of": primary.name,
            })
            self.ledger.event(
                "fallback_executed",
                f"Fallback '{alt_name}' acionado após falha de '{primary.name}'.",
                primario=primary.name, alternativo=alt_name, valor=value,
            )
            return True
        self.ledger.alert(
            f"Todos os fallbacks de '{primary.name}' falharam.",
            motivo="fallback_esgotado", ator=primary.name,
        )
        return False

    # Efeitos físicos determinísticos (PLAN §6.5: respostas plausíveis)
    def _power_cap_effect(self, value):
        if value < self.cpu_power:
            self.cpu_power = float(value)

    def _fan_effect(self, value):
        # resfriamento proporcional ao PWM (determinístico)
        self.set_sensor("cpu_temp", max(0.0, self.cpu_temperature - (value / 255.0) * 8.0))

    # ------------------------------------------------------------------
    # Avanço do tick (chamado uma vez por tick do runtime)
    # ------------------------------------------------------------------
    def update_hardware_state(self):
        """Aplica o cronograma do tick corrente. Sem aleatoriedade."""
        self.ticks += 1
        for name, value in self.timeline.get(self.ticks, {}).items():
            self.set_sensor(name, value)
