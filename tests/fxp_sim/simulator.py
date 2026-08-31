# -*- coding: utf-8 -*-
"""Simulador físico determinístico do FXP — esqueleto da Etapa 1 (PLAN §6.5).

Papel na suíte: é o "mundo externo" roteirizado que o runtime consulta a cada
tick. Evolui para um módulo separado do FXP real na Etapa 3 (PLAN §3); aqui
ficam apenas as capacidades exigidas pelos cenários de teste:

- Séries temporais roteirizadas (`programar`/`set_sensor`) — determinismo total,
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
VALOR_POETICO_CANONICO = "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"


@dataclass
class SensorEstado:
    nome: str
    grandeza: str          # temperatura | potencia | atencao | ...
    unidade: str           # °C | W | %
    faixa: tuple[float, float]
    valor: float
    registrado: bool = True
    acessivel: bool = True


@dataclass
class AtorEstado:
    nome: str
    funcao: str
    minimo: float | None = None
    maximo: float | None = None
    safety_limit: float | None = None
    atual: object = None
    disponivel: bool = True
    fallback: list[str] = field(default_factory=list)
    aplicar: object = None  # callable(valor) -> None | None (efeito simulado)


class FXPSimulator:
    """Barramento FXP simulado, determinístico, para a suíte da Etapa 1."""

    def __init__(self, caderno, tick_seconds: float = 1.0):
        self.caderno = caderno          # classe Caderno do protótipo (injetada)
        self.tick_seconds = tick_seconds
        self.ticks = 0
        self.disk_bytes_used = 1024

        self.valores: dict[str, float] = {
            "cpu_temp": 55.0,
            "cpu_power": 150.0,
            "attention": 100.0,
        }
        self.sensores: dict[str, SensorEstado] = {}
        self._registrar_padrao("cpu_temp", "temperatura", "°C", (0, 120))
        self._registrar_padrao("cpu_power", "potencia", "W", (0, 500))
        self._registrar_padrao("attention", "atencao", "%", (0, 100))

        self.atores: dict[str, AtorEstado] = {}
        self._registrar_atores_padrao()

        # Mensagens FXP serializadas (fronteira em processo, sem schema binário)
        self.outbox: list[dict] = []
        self.entregues: list[dict] = []
        self._seq = 0

        # Séries roteirizadas: {tick: {"cpu_temp": 90.0, ...}}
        self.cronograma: dict[int, dict[str, float]] = {}

    # ------------------------------------------------------------------
    # Registro (FORMAL §6)
    # ------------------------------------------------------------------
    def _registrar_padrao(self, nome, grandeza, unidade, faixa):
        self.sensores[nome] = SensorEstado(
            nome=nome,
            grandeza=grandeza,
            unidade=unidade,
            faixa=faixa,
            valor=self.valores[nome],
        )

    def _registrar_atores_padrao(self):
        """Registro mínimo obrigatório (FORMAL §6)."""
        self.registrar_ator(
            "CpuPowerCap", "limite de potência da CPU (W)",
            minimo=10, maximo=250, safety_limit=200,
            aplicar=lambda v: self._efeito_power_cap(v),
        )
        self.registrar_ator(
            "Ventoinha", "velocidade da ventoinha (PWM)",
            minimo=0, maximo=255, safety_limit=200,
            aplicar=lambda v: self._efeito_ventoinha(v),
        )
        self.registrar_ator("LedIndicador", "estado textual do LED")

    def registrar_ator(self, nome, funcao, minimo=None, maximo=None,
                       safety_limit=None, aplicar=None):
        self.atores[nome] = AtorEstado(
            nome=nome, funcao=funcao, minimo=minimo, maximo=maximo,
            safety_limit=safety_limit, atual=None, disponivel=True,
            fallback=[], aplicar=aplicar,
        )

    def definir_fallback(self, primario: str, *alternativos: str):
        """Política de fallback fica no REGISTRO do FXP (FORMAL §4.3)."""
        self.atores[primario].fallback = list(alternativos)

    def registrar_sensor(self, nome, grandeza="generico", unidade="",
                         faixa=(0.0, 1e9), valor=0.0):
        self.valores[nome] = float(valor)
        self.sensores[nome] = SensorEstado(nome, grandeza, unidade, faixa, float(valor))

    def desregistrar_sensor(self, nome):
        """Simula sensor não registrado (falha de I/O — FORMAL §4.7)."""
        self.sensores.pop(nome, None)

    # ------------------------------------------------------------------
    # Roteirização do mundo e injeção de falhas
    # ------------------------------------------------------------------
    def set_sensor(self, nome: str, valor: float):
        if nome not in self.sensores:
            raise KeyError(f"sensor '{nome}' não registrado no simulador")
        self.sensores[nome].valor = float(valor)
        if nome in self.valores or nome in ("cpu_temp", "cpu_power", "attention"):
            self.valores[nome] = float(valor)

    def falhar_sensor(self, nome: str):
        """Sensor registrado porém inacessível (FORMAL §4.7)."""
        self.sensores[nome].acessivel = False

    def programar(self, tick: int, **valores: float):
        """Agenda valores absolutos de sensores para um tick (1-based)."""
        self.cronograma.setdefault(tick, {}).update(valores)

    def falhar_ator(self, nome: str):
        """Ator para de responder (heartbeat falho — BDD Caso 3)."""
        self.atores[nome].disponivel = False

    def recuperar_ator(self, nome: str):
        self.atores[nome].disponivel = True

    # ------------------------------------------------------------------
    # Propriedades usadas pelo runtime
    # ------------------------------------------------------------------
    @property
    def cpu_power(self) -> float:
        return self.valores["cpu_power"]

    @cpu_power.setter
    def cpu_power(self, valor: float):
        self.valores["cpu_power"] = float(valor)
        if "cpu_power" in self.sensores:
            self.sensores["cpu_power"].valor = float(valor)

    @property
    def cpu_temperature(self) -> float:
        return self.valores["cpu_temp"]

    @cpu_temperature.setter
    def cpu_temperature(self, valor: float):
        self.set_sensor("cpu_temp", valor)

    @property
    def human_attention(self) -> float:
        return self.valores["attention"]

    @human_attention.setter
    def human_attention(self, valor: float):
        self.set_sensor("attention", valor)

    # ------------------------------------------------------------------
    # Interface consultada pelo runtime
    # ------------------------------------------------------------------
    def read_sensor(self, name: str, timeout_s: float = 0.001) -> float | None:
        """Leitura por nome simbólico. Falha de I/O -> None + alerta (§4.7).

        NUNCA retorna 0.0 para sensor ausente: zero é leitura física válida
        e dispararia falsas condições de revisão (FORMAL §4.7).
        """
        sensor = self.sensores.get(name)
        if sensor is None or not sensor.registrado:
            self.caderno.alert(
                f"Sensor '{name}' não registrado no FXP (falha de I/O). "
                f"Condição não avaliada neste tick.",
                motivo="sensor_nao_registrado", sensor=name,
            )
            return None
        if not sensor.acessivel:
            self.caderno.alert(
                f"Sensor '{name}' registrado porém inacessível (falha de leitura). "
                f"Condição não avaliada neste tick.",
                motivo="sensor_inacessivel", sensor=name,
            )
            return None
        return round(float(sensor.valor), 2)

    def act(self, actor_name: str, value) -> bool:
        """Comando a ator: serializa, valida limites (inclusivos), entrega.

        FORMAL §4.3: fora dos limites -> rejeitado SEM envio, registrado como
        `actor_rejected_value`; ator indisponível -> política de fallback do
        registro; tentativa, falha e fallback aparecem no Caderno.
        """
        self._seq += 1
        mensagem = {
            "seq": self._seq,
            "op": "act",
            "ator": actor_name,
            "valor": value,
            "tick": self.ticks,
        }
        self.outbox.append(mensagem)

        ator = self.atores.get(actor_name)
        if ator is None:
            self.caderno.event(
                "ator_inexistente",
                f"Ator '{actor_name}' não registrado no FXP.",
                ator=actor_name,
            )
            self.caderno.actuator_action(actor_name, value, False)
            return False

        if not ator.disponivel:
            self.caderno.actuator_action(actor_name, value, False)
            self.caderno.event(
                "ator_indisponivel",
                f"Heartbeat do ator '{actor_name}' não respondeu.",
                ator=actor_name,
            )
            return self._tentar_fallback(ator, value)

        violacao = self._violacao_de_limite(ator, value)
        if violacao is not None:
            limite, valor_limite = violacao
            self.caderno.event(
                "actor_rejected_value",
                f"Comando a '{actor_name}' rejeitado sem envio: valor {value} "
                f"viola {limite} = {valor_limite}.",
                ator=actor_name, valor=value,
                limite=limite, limite_valor=valor_limite,
            )
            return False

        self._entregar(ator, value, mensagem)
        return True

    # ------------------------------------------------------------------
    # Internos
    # ------------------------------------------------------------------
    @staticmethod
    def _violacao_de_limite(ator: AtorEstado, value):
        """Limites inclusivos: valor igual ao limite é aceito (FORMAL §4.3)."""
        if ator.minimo is not None and value < ator.minimo:
            return ("min", ator.minimo)
        if ator.maximo is not None and value > ator.maximo:
            return ("max", ator.maximo)
        if ator.safety_limit is not None and value > ator.safety_limit:
            return ("safety_limit", ator.safety_limit)
        return None

    def _entregar(self, ator: AtorEstado, value, mensagem: dict):
        if ator.aplicar is not None:
            ator.aplicar(value)
        ator.atual = value
        self.entregues.append({**mensagem, "evento": "entrega"})
        self.caderno.actuator_action(ator.nome, value, True)

    def _tentar_fallback(self, primario: AtorEstado, value) -> bool:
        for nome_alt in primario.fallback:
            alternativo = self.atores.get(nome_alt)
            if alternativo is None or not alternativo.disponivel:
                continue
            if self._violacao_de_limite(alternativo, value) is not None:
                self.caderno.alert(
                    f"Fallback '{nome_alt}' rejeitou o valor {value} (limites).",
                    motivo="fallback_rejeitado", ator=nome_alt,
                )
                continue
            self._entregar(alternativo, value, {
                "seq": self._seq, "op": "act", "ator": nome_alt,
                "valor": value, "tick": self.ticks, "fallback_de": primario.nome,
            })
            self.caderno.event(
                "fallback_executado",
                f"Fallback '{nome_alt}' acionado após falha de '{primario.nome}'.",
                primario=primario.nome, alternativo=nome_alt, valor=value,
            )
            return True
        self.caderno.alert(
            f"Todos os fallbacks de '{primario.nome}' falharam.",
            motivo="fallback_esgotado", ator=primario.nome,
        )
        return False

    # Efeitos físicos determinísticos (PLAN §6.5: respostas plausíveis)
    def _efeito_power_cap(self, valor):
        if valor < self.cpu_power:
            self.cpu_power = float(valor)

    def _efeito_ventoinha(self, valor):
        # resfriamento proporcional ao PWM (determinístico)
        self.set_sensor("cpu_temp", max(0.0, self.cpu_temperature - (valor / 255.0) * 8.0))

    # ------------------------------------------------------------------
    # Avanço do tick (chamado uma vez por tick do runtime)
    # ------------------------------------------------------------------
    def update_hardware_state(self):
        """Aplica o cronograma do tick corrente. Sem aleatoriedade."""
        self.ticks += 1
        for nome, valor in self.cronograma.get(self.ticks, {}).items():
            self.set_sensor(nome, valor)
