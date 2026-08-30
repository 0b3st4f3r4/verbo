# -*- coding: utf-8 -*-
"""
================================================================================
                VERBOLANG ENGINE & RUNTIME BLUEPRINT
================================================================================
Protótipo emulado em Python 3 que serve como especificação de engenharia
e blueprint estrutural para portabilidade da VerboLang para Rust ou C.

Inclui:
- FXP (Flux Protocol) unificando sensores (entrada) e atores (saída).
- Relógio virtual para simulação determinística.
- Caderno (log termodinâmico) registrando vazamentos, leituras e atuações,
  com cadeia de integridade SHA-256 e exportação JSONL.
- Bloco main com every/keep/act (intérprete de AST simplificada).
- Semântica de subvert conforme docs/FORMAL.md §4.5: interrompe a forma no
  mesmo tick sem cancelar as ações seguintes da mesma regra.
- Conjugações: event, equilibrium, nonequilibrium.
- Operador poético subvert().
- Condições de revisão que podem referenciar múltiplos sensores por forma.
- Ações com atores via act(nome, valor).

Copyright (C) 2026 Silvano Neto

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, version 3 of the License. This program is distributed in the hope
that it will be useful, but WITHOUT ANY WARRANTY; without even the implied
warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
General Public License for more details (LICENSE na raiz do repositório).
================================================================================
"""

import asyncio
import hashlib
import json
import random
from collections.abc import Callable
from typing import ClassVar, final

# Códigos de cor ANSI para o Caderno
RESET = "\033[0m"
BOLD = "\033[1m"
CYAN = "\033[36m"
GREEN = "\033[32m"
YELLOW = "\033[33m"
RED = "\033[31m"
MAGENTA = "\033[35m"
WHITE = "\033[37m"


# ==============================================================================
# 1. FXP (FLUX PROTOCOL) — Camada de I/O com Sensores e Atores
# ==============================================================================
@final
class FXP:
    """
    Simula o barramento de I/O que unifica sensores (entrada) e atores (saída).
    Em Rust/C, esta classe interage com sysfs, ioctl, sockets, GPIO, etc.
    Mantém um registro de sensores e atores disponíveis.
    """

    def __init__(self):
        # Estado interno simulado dos sensores físicos
        self.cpu_temperature = 55.0  # Celsius
        self.cpu_power = 150.0  # Watts
        self.human_attention = 100.0  # Porcentagem (0-100)
        self.disk_bytes_used = 1024  # Bytes em armazenamento estável
        self.solar_generation = 120.0  # Watts gerados pela placa solar

        # Registro de sensores (nome simbólico -> função de leitura)
        # Pode ser estendido com novos sensores sem alterar o runtime.
        self.sensors: dict[str, Callable[[], float]] = {
            "cpu_power": lambda: self.cpu_power,
            "cpu_temp": lambda: self.cpu_temperature,
            "attention": lambda: self.human_attention,
            "solar_panel": lambda: self.solar_generation,
            "disk_bytes": lambda: float(self.disk_bytes_used),
        }

        # Registro de atores (nome simbólico -> dicionário com propriedades)
        # Cada ator tem limites e um método 'apply' que simula seu efeito.
        self.actors: dict[str, dict] = {
            "CpuPowerCap": {
                "min": 10.0,
                "max": 250.0,
                "safety_limit": 200.0,
                "current": 150.0,
                "apply": self._actor_cpu_power_cap,
                "description": "Limite de potência da CPU (Watts)",
            },
            "Ventoinha": {
                "min": 0,
                "max": 255,
                "safety_limit": 200,
                "current": 100,
                "apply": self._actor_fan_speed,
                "description": "Velocidade da ventoinha (PWM)",
            },
            "LedIndicador": {
                "min": None,
                "max": None,
                "safety_limit": None,
                "current": "desligado",
                "apply": self._actor_led,
                "description": "LED indicador (string)",
            },
        }

    # ---------- Métodos de atualização do estado simulado ----------
    def update_hardware_state(self):
        """
        Atualiza dinamicamente as métricas com flutuações físicas.
        Chamado uma vez por tick do engine.
        """
        if self.cpu_power > 300.0:
            self.cpu_temperature += random.uniform(2.0, 5.0)
        elif self.cpu_power < 50.0:
            self.cpu_temperature -= random.uniform(1.5, 3.0)
        else:
            self.cpu_temperature += random.uniform(-1.0, 1.0)

        self.cpu_temperature = max(45.0, min(100.0, self.cpu_temperature))
        self.solar_generation = max(
            0.0, self.solar_generation + random.uniform(-15.0, 15.0)
        )

    # ---------- Leitura de sensores ----------
    def read_sensor(self, name: str, timeout_s: float = 0.001) -> float | None:
        """
        Lê um sensor pelo nome EXCLUSIVAMENTE simbólico (docs/FORMAL.md §3:
        caminhos de sistema operacional não são nomes válidos de sensor).

        `timeout_s` documenta o contrato de não bloqueio do FXP (AGENTS.md,
        EIF): neste protótipo a leitura é um lookup em memória; em Rust/C a
        leitura é assíncrona com timeout e o parâmetro é honrado.

        Retorna None se o sensor não estiver registrado — falha de I/O,
        alertada no Caderno (docs/FORMAL.md §4.7). NUNCA retorna 0.0 para sensor
        ausente: zero é leitura física válida e dispararia falsas revisões.
        """
        if name not in self.sensors:
            Caderno.alert(f"Sensor '{name}' não registrado no FXP (falha de I/O).")
            return None
        return round(self.sensors[name](), 2)

    # ---------- Atuação sobre atores ----------
    def act(self, actor_name: str, value) -> bool:
        """
        Envia um comando para um ator. Valida limites e aplica.
        Retorna True se sucesso, False se falha.
        """
        if actor_name not in self.actors:
            print(f"{RED}[FXP] Ator '{actor_name}' não encontrado!{RESET}")
            return False

        actor = self.actors[actor_name]
        # Validação de limites (se definidos)
        if actor["min"] is not None and value < actor["min"]:
            print(
                f"{YELLOW}[FXP] Valor {value} abaixo do mínimo para {actor_name}.{RESET}"
            )
            return False
        if actor["max"] is not None and value > actor["max"]:
            print(
                f"{YELLOW}[FXP] Valor {value} acima do máximo para {actor_name}.{RESET}"
            )
            return False
        if actor["safety_limit"] is not None and value > actor["safety_limit"]:
            print(
                f"{RED}[FXP] Valor {value} excede safety_limit de {actor_name}! Bloqueado.{RESET}"
            )
            return False

        # Aplica o comando
        actor["apply"](value)
        actor["current"] = value
        print(f"{CYAN}[FXP] Ator '{actor_name}' ajustado para {value}.{RESET}")
        return True

    # ---------- Implementações simuladas dos atores ----------
    def _actor_cpu_power_cap(self, value: float):
        """Limita a potência máxima da CPU (simulado)."""
        if value < self.cpu_power:
            self.cpu_power = value

    def _actor_fan_speed(self, value: int):
        """Ajusta a velocidade da ventoinha (simulado: reduz temperatura)."""
        cooling_effect = (100 - value) * 0.05  # ex: 200 -> -5°C
        self.cpu_temperature += cooling_effect

    def _actor_led(self, value: str):
        """Acende LED com cor/estado (simulado)."""
        pass


# ==============================================================================
# 2. O CADERNO (SISTEMA DE AUDITORIA TERMODINÂMICA)
# ==============================================================================
class Caderno:
    """
    Logger termodinâmico: registra eventos, vazamentos, leituras e atuações.

    Integridade (AGENTS.md, AC): cada evento é encadeado por SHA-256
    (hash_n = SHA-256(hash_{n-1} || evento_n)), formando uma cadeia à prova
    de adulteração — qualquer edição retroativa quebra a cadeia e é
    detectada por verify_chain(). Em Rust/C os eventos são gravados de
    forma assíncrona (buffer + flush periódico) em formato binário compacto
    (Cap'n Proto / FlatBuffers); este protótipo exporta JSONL para que um
    agente externo possa auditar recomputando a cadeia a partir do arquivo.
    """

    _events: ClassVar[list[dict]] = []
    _chain_head: ClassVar[str] = "0" * 64

    @classmethod
    def _record(cls, kind: str, msg: str, **extra) -> dict:
        seq = len(cls._events)
        line = f"{seq}\x1f{kind}\x1f{msg}"
        if extra:
            line += "\x1f" + json.dumps(extra, sort_keys=True, ensure_ascii=False)
        entry = {
            "seq": seq,
            "kind": kind,
            "msg": msg,
            **extra,
            "hash": hashlib.sha256(
                (cls._chain_head + line).encode("utf-8")
            ).hexdigest(),
        }
        cls._events.append(entry)
        cls._chain_head = entry["hash"]
        return entry

    @classmethod
    def chain_head(cls) -> str:
        return cls._chain_head

    @classmethod
    def verify_chain(cls) -> bool:
        """Recomputa a cadeia SHA-256 a partir dos eventos e confere a cabeça."""
        head = "0" * 64
        for e in cls._events:
            line = f"{e['seq']}\x1f{e['kind']}\x1f{e['msg']}"
            extra = {
                k: v for k, v in e.items() if k not in ("seq", "kind", "msg", "hash")
            }
            if extra:
                line += "\x1f" + json.dumps(extra, sort_keys=True, ensure_ascii=False)
            head = hashlib.sha256((head + line).encode("utf-8")).hexdigest()
            if head != e["hash"]:
                return False
        return head == cls._chain_head

    @classmethod
    def export_jsonl(cls, path: str = "caderno_log.jsonl") -> str:
        """Exporta o log (evento + hash por linha) para auditoria externa."""
        with open(path, "w", encoding="utf-8") as f:
            for e in cls._events:
                f.write(json.dumps(e, ensure_ascii=False) + "\n")
        return path

    @staticmethod
    def info(msg: str):
        Caderno._record("INFO", msg)
        print(f"{GREEN}[CADERNO - INFO]{RESET} {msg}")

    @staticmethod
    def warn(msg: str):
        Caderno._record("AVALIACAO", msg)
        print(f"{YELLOW}[CADERNO - AVALIAÇÃO]{RESET} {msg}")

    @staticmethod
    def alert(msg: str):
        Caderno._record("ALERTA", msg)
        print(f"{RED}[CADERNO - REVISÃO COGNITIVA/ALERTA]{RESET} {msg}")

    @staticmethod
    def colapso(msg: str):
        Caderno._record("COLAPSO", msg)
        print(f"{RED}{BOLD}[CADERNO - COLAPSO TERMODINÂMICO]{RESET} {msg}")

    @staticmethod
    def art(msg: str):
        Caderno._record("SUBVERSAO", msg)
        print(f"{MAGENTA}{BOLD}[CADERNO - SUBVERSÃO POÉTICA]{RESET} {msg}")

    @staticmethod
    def leak(form_name: str, power_watts: float, duration_seconds: float):
        joules = power_watts * duration_seconds
        Caderno._record(
            "VAZAMENTO",
            f"Forma '{form_name}' dissipou {joules:.2f} Joules "
            f"({power_watts:.2f} W por {duration_seconds:.2f}s)",
            forma=form_name,
            watts=power_watts,
            segundos=duration_seconds,
            joules=round(joules, 2),
        )
        print(
            f"{CYAN}[CADERNO - VAZAMENTO]{RESET} Forma '{form_name}' dissipou {joules:.2f} Joules ({power_watts:.2f} W por {duration_seconds:.2f}s)"
        )

    @staticmethod
    def sensor_read(sensor_name: str, value: float):
        Caderno._record(
            "LEITURA",
            f"Sensor '{sensor_name}' = {value}",
            sensor=sensor_name,
            valor=value,
        )
        print(f"{WHITE}[CADERNO - LEITURA]{RESET} Sensor '{sensor_name}' = {value}")

    @staticmethod
    def actuator_action(actor_name: str, value, success: bool):
        status = "sucesso" if success else "falha"
        Caderno._record(
            "ATUACAO",
            f"Ator '{actor_name}' <- {value} ({status})",
            ator=actor_name,
            valor=value,
            sucesso=success,
        )
        print(
            f"{WHITE}[CADERNO - ATUAÇÃO]{RESET} Ator '{actor_name}' <- {value} ({status})"
        )


# ==============================================================================
# 3. CONJUGAÇÕES (KINETIC STATES) E ESTRUTURA DE FORMAS
# ==============================================================================
class Form:
    """Estrutura base que representa um recorte do fluxo contínuo do Real."""

    def __init__(
        self,
        name: str,
        value,
        horizon: float,
        currency: str,
        source_path: str,
        classification: str,
        current_time: float,
    ):
        self.name = name
        self.value = value
        self.horizon = horizon
        self.creation_time = current_time  # tempo virtual de criação
        self.currency = currency
        self.source_path = source_path  # nome simbólico do sensor principal
        self.classification_currency = classification
        self.review_conditions: list[dict] = []
        self.is_dissolved = False

    def add_review_condition(
        self,
        sensor_var: str,
        op: str,
        threshold: float,
        actions: list[dict],
    ):
        """
        Adiciona uma condição de revisão.
        sensor_var: nome do sensor a monitorar (pode ser qualquer sensor do FXP,
                    não necessariamente o source_path).
        op: operador de comparação ('<', '>', '<=', '>=', '==', '!=').
        threshold: valor limiar.
        actions: lista de ações a executar quando a condição for verdadeira.
                 Cada ação é um dicionário com 'action' e argumentos opcionais.
        """
        self.review_conditions.append(
            {
                "sensor": sensor_var,
                "op": op,
                "threshold": threshold,
                "actions": actions,
            }
        )

    def check_horizon(self, current_time: float) -> bool:
        return (current_time - self.creation_time) >= self.horizon


class EventForm(Form):
    """Conjugação 'event': transitória, horizonte curto, sem manutenção."""

    def __init__(
        self,
        name: str,
        value,
        horizon: float,
        source_path: str,
        current_time: float,
    ):
        super().__init__(
            name, value, horizon, "CpuCycles", source_path, "Transiente", current_time
        )


class EquilibriumForm(Form):
    """Conjugação 'equilibrium': persistente, sem manutenção, com custo em bytes."""

    def __init__(
        self,
        name: str,
        value,
        horizon: float,
        source_path: str,
        cost_bytes: int,
        current_time: float,
    ):
        super().__init__(
            name,
            value,
            horizon,
            "DiskBytes",
            source_path,
            "EstabilidadePersistente",
            current_time,
        )
        self.cost_bytes = cost_bytes


class NonequilibriumForm(Form):
    """Conjugação 'nonequilibrium': laborativa, requer keep() contínuo."""

    def __init__(
        self,
        name: str,
        value,
        horizon: float,
        source_path: str,
        maintenance_deadline: float,
        exchange_mode: str,
        current_time: float,
    ):
        super().__init__(
            name,
            value,
            horizon,
            "PowerWatts",
            source_path,
            "TrabalhoAtivo",
            current_time,
        )
        self.maintenance_deadline = maintenance_deadline
        self.last_maintenance = current_time
        self.exchange_mode = exchange_mode

    def keep(self, current_time: float):
        self.last_maintenance = current_time

    def check_maintenance_timeout(self, current_time: float) -> bool:
        return (current_time - self.last_maintenance) > self.maintenance_deadline


# ==============================================================================
# 4. RUNTIME ENGINE: O GOVERNO DO MOVIMENTO
# ==============================================================================
class VerboLangEngine:
    """Loop de processamento central: consulta FXP, avalia condições, atua e audita."""

    def __init__(self):
        self.fxp = FXP()
        self.forms: dict[str, Form] = {}
        self.clock = 0
        self.sim_time = 0.0  # relógio virtual em segundos

    def register_form(self, form: Form):
        self.forms[form.name] = form
        Caderno.info(f"Forma '{form.name}' conjugada no sistema.")

    def dissolve_form(self, name: str):
        if name in self.forms:
            self.forms[name].is_dissolved = True
            Caderno.info(
                f"{WHITE}ALÍVIO TERMODINÂMICO{RESET} -> Forma '{name}' dissolvida."
            )
            del self.forms[name]

    def subvert_form(self, name: str):
        if name in self.forms:
            form = self.forms[name]
            form.value = "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"
            # Reduz o consumo elétrico simulado para resfriar o sistema
            self.fxp.cpu_power = 15.0
            Caderno.art(
                f"Operador subvert() invocado na forma '{name}'! Acumulação abortada."
            )
            Caderno.info(f"Novo valor de '{name}': '{form.value}'")

    def eval_condition(self, sensor_val: float, op: str, threshold: float) -> bool:
        if op == "<":
            return sensor_val < threshold
        if op == ">":
            return sensor_val > threshold
        if op == "<=":
            return sensor_val <= threshold
        if op == ">=":
            return sensor_val >= threshold
        if op == "==":
            return sensor_val == threshold
        if op == "!=":
            return sensor_val != threshold
        return False

    def execute_actions(self, form: Form, actions: list[dict]) -> bool:
        """
        Executa a lista de ações associadas a uma condição disparada.
        Retorna True se a forma foi dissolvida ou reclassificada (deve interromper).
        """
        doomed = False  # subvert marca a forma para dissolução no mesmo tick
        for act in actions:
            action_type = act.get("action")
            if action_type == "dissolve":
                self.dissolve_form(form.name)
                return True
            elif action_type == "subvert":
                # docs/FORMAL.md §4.5: subvert substitui o valor lógico e marca a
                # forma para dissolução NO MESMO tick (interrupção de
                # prioridade máxima), porém NÃO cancela as ações seguintes da
                # mesma regra — em particular, qualquer act() associado é
                # enviado ao FXP.
                self.subvert_form(form.name)
                doomed = True
            elif action_type == "reclassify_as_equilibrium":
                # Persiste em disco (simulado) e converte para EquilibriumForm.
                # horizon/cost_bytes fixos: atalhos do protótipo — a renovação
                # de horizonte na reclassificação será definida na Etapa 2.
                self.fxp.disk_bytes_used += 1024  # simula escrita
                new_form = EquilibriumForm(
                    name=form.name,
                    value=form.value,
                    horizon=10.0,
                    source_path="disk_bytes",  # nome simbólico registrado no FXP
                    cost_bytes=1024,
                    current_time=self.sim_time,
                )
                new_form.review_conditions = form.review_conditions.copy()
                self.forms[form.name] = new_form
                Caderno.info(
                    f"Forma '{form.name}' reclassificada para 'equilibrium' (persistida)."
                )
                return True
            elif action_type == "reclassify_as_nonequilibrium":
                # horizon/maintenance_deadline fixos: atalhos do protótipo (Etapa 2).
                new_form = NonequilibriumForm(
                    name=form.name,
                    value=form.value,
                    horizon=60.0,
                    source_path="attention",
                    maintenance_deadline=3.0,
                    exchange_mode="cooperation",
                    current_time=self.sim_time,
                )
                new_form.review_conditions = form.review_conditions.copy()
                self.forms[form.name] = new_form
                Caderno.info(
                    f"Forma '{form.name}' reclassificada para 'nonequilibrium' (trabalho ativo)."
                )
                return True
            elif action_type == "notify_shutdown":
                Caderno.warn(
                    f"Interrupção do sistema! Desligando cargas secundárias ligadas a '{form.name}'."
                )
                # Não interrompe outras ações nem dissolve a forma
            elif action_type == "act":
                actor_name = act.get("actor")
                value = act.get("value")
                success = self.fxp.act(actor_name, value)
                Caderno.actuator_action(actor_name, value, success)
                if not success:
                    Caderno.alert(
                        f"Falha na atuação do ator '{actor_name}' para a forma '{form.name}'."
                    )
            else:
                Caderno.warn(f"Ação desconhecida '{action_type}' ignorada.")
        if doomed:
            # Dissolução da forma subvertida dentro do mesmo tick (≤ 1 tick virtual)
            self.dissolve_form(form.name)
            return True
        return False

    def tick(self):
        """Avança um segundo virtual e processa todas as formas."""
        self.clock += 1
        self.sim_time += 1.0
        print(f"\n{BOLD}--- SEGUNDO {self.clock} ---{RESET}")

        # Atualiza o estado do hardware uma única vez por tick
        self.fxp.update_hardware_state()

        # Itera sobre uma cópia das chaves para permitir remoção segura
        for name in list(self.forms.keys()):
            if name not in self.forms:
                continue

            form = self.forms[name]

            # 1. Auditoria de vazamento energético
            power_consumption = (
                self.fxp.cpu_power if form.currency == "PowerWatts" else 5.0
            )
            Caderno.leak(form.name, power_consumption, 1.0)

            # 2. Leitura do sensor associado à forma (source_path) e registro
            sensor_value = self.fxp.read_sensor(form.source_path)
            if sensor_value is not None:
                Caderno.sensor_read(form.source_path, sensor_value)

            # 3. Avaliação das condições de revisão
            #    Cada condição pode usar um sensor diferente.
            condition_triggered = False
            for cond in form.review_conditions:
                current_sensor_val = self.fxp.read_sensor(cond["sensor"])
                if current_sensor_val is None:
                    # Falha de I/O já alertada; condição não avaliada (docs/FORMAL.md §4.7)
                    continue
                Caderno.sensor_read(cond["sensor"], current_sensor_val)
                if self.eval_condition(
                    current_sensor_val, cond["op"], cond["threshold"]
                ):
                    Caderno.alert(
                        f"Condição de revisão disparada para '{form.name}': "
                        f"{cond['sensor']} {cond['op']} {cond['threshold']} (lido: {current_sensor_val})"
                    )
                    # Executa as ações associadas
                    if self.execute_actions(form, cond["actions"]):
                        condition_triggered = True
                        break  # sai do loop de condições
            if condition_triggered or name not in self.forms:
                continue

            # 4. Verificação de manutenção (apenas nonequilibrium)
            if isinstance(form, NonequilibriumForm):
                if form.check_maintenance_timeout(self.sim_time):
                    Caderno.colapso(
                        f"Prazo de manutenção de '{form.name}' expirou! "
                        f"(sem keep() por {form.maintenance_deadline}s)"
                    )
                    self.dissolve_form(form.name)
                    continue

            # 5. Verificação de horizonte
            if form.check_horizon(self.sim_time):
                Caderno.warn(
                    f"Horizonte de validade de '{form.name}' esgotou-se. Dissolvendo."
                )
                self.dissolve_form(form.name)


# ==============================================================================
# 4.1 BLOCO main: INTÉRPRETE DE every / keep / act
# ==============================================================================
class MainInterpreter:
    """
    Interpreta o bloco `main` da docs/FORMAL.md §3 (statements keep, act, every).
    O protótipo recebe o main como estrutura de dados (AST simplificada);
    o parser de arquivos .vl que produz essa AST é entregável da Etapa 2
    (cf. PLAN.md §2.1).
    """

    def __init__(self, engine: VerboLangEngine):
        self.engine = engine
        self.every_blocks: list[dict] = []

    def add_every(self, period: float, statements: list[dict]):
        """Equivalente a `every <period> { <statements> }` dentro do main."""
        self.every_blocks.append(
            {"period": period, "statements": statements, "next_due": period}
        )

    def run_due(self):
        """Executa os blocos `every` vencidos; chamado uma vez por tick."""
        now = self.engine.sim_time
        for block in self.every_blocks:
            if now + 1e-9 >= block["next_due"]:
                for st in block["statements"]:
                    self._run_statement(st)
                block["next_due"] += block["period"]

    def _run_statement(self, st: dict):
        kind = st.get("statement")
        if kind == "keep":
            form = self.engine.forms.get(st["form"])
            if isinstance(form, NonequilibriumForm):
                form.keep(self.engine.sim_time)
        elif kind == "act":
            ok = self.engine.fxp.act(st["actor"], st["value"])
            Caderno.actuator_action(st["actor"], st["value"], ok)
        else:
            Caderno.warn(f"Statement desconhecido no bloco main: '{kind}' ignorado.")


# ==============================================================================
# 5. SIMULAÇÃO PRÁTICA COMPLETA
# ==============================================================================
async def main():
    engine = VerboLangEngine()

    # Bloco `main` da linguagem (estrutura cf. docs/FORMAL.md §3; adaptação do
    # Exemplo 4 — canônico: every 4s { keep(TarefaImportante) } — para as
    # três formas conjuradas abaixo):
    #   every 1s  { keep(PensarLivre), keep(TradingEspeculativo), keep(ServidorCritico) }
    #   every 10s { act(LedIndicador, "verde") }
    main_block = MainInterpreter(engine)
    main_block.add_every(
        1.0,
        [
            {"statement": "keep", "form": "PensarLivre"},
            {"statement": "keep", "form": "TradingEspeculativo"},
            {"statement": "keep", "form": "ServidorCritico"},
        ],
    )
    main_block.add_every(
        10.0, [{"statement": "act", "actor": "LedIndicador", "value": "verde"}]
    )

    # --- CONJURAÇÃO 1: PensarLivre (nonequilibrium) ---
    # Sensor principal: attention
    # Condição adicional: usa o mesmo sensor, mas poderia usar outro.
    pensar_livre = NonequilibriumForm(
        name="PensarLivre",
        value="consciencia_anteneoliberal_ativa",
        horizon=60.0,
        source_path="attention",
        maintenance_deadline=3.0,
        exchange_mode="cooperation",
        current_time=engine.sim_time,
    )
    pensar_livre.add_review_condition(
        "attention", "<", 30.0, [{"action": "reclassify_as_equilibrium"}]
    )
    engine.register_form(pensar_livre)

    # --- CONJURAÇÃO 2: TradingEspeculativo (nonequilibrium) ---
    # Sensor principal: cpu_temp
    # Condição adicional: usa cpu_temp e também ação com ator.
    trading_predatorio = NonequilibriumForm(
        name="TradingEspeculativo",
        value="lucro_arbitragem_alta_frequencia",
        horizon=7.0,
        source_path="cpu_temp",
        maintenance_deadline=2.0,
        exchange_mode="extraction",
        current_time=engine.sim_time,
    )
    trading_predatorio.add_review_condition(
        "cpu_temp",
        ">",
        85.0,
        [{"action": "subvert"}, {"action": "act", "actor": "CpuPowerCap", "value": 50}],
    )
    engine.register_form(trading_predatorio)

    # --- CONJURAÇÃO 3: ServidorCritico (nonequilibrium) ---
    # Sensor principal: cpu_temp (cf. docs/FORMAL.md Exemplo 3)
    # Condições adicionais: usa cpu_temp e attention (múltiplos sensores por forma!)
    servidor_critico = NonequilibriumForm(
        name="ServidorCritico",
        value="processamento_contínuo",
        horizon=3600.0,
        source_path="cpu_temp",
        maintenance_deadline=10.0,
        exchange_mode="cooperation",
        current_time=engine.sim_time,
    )
    servidor_critico.add_review_condition(
        "cpu_temp", ">", 70.0, [{"action": "act", "actor": "Ventoinha", "value": 200}]
    )
    servidor_critico.add_review_condition(
        "attention", "<", 20.0, [{"action": "notify_shutdown"}]
    )
    engine.register_form(servidor_critico)

    # Demonstração do FXP: leitura de sensor e atuação
    Caderno.info(
        f"Leitura inicial do sensor 'cpu_temp': {engine.fxp.read_sensor('cpu_temp')}°C"
    )
    Caderno.info(
        f"Atuação inicial no ator 'Ventoinha': {engine.fxp.act('Ventoinha', 150)}"
    )

    # --- LOOP DE SIMULAÇÃO (12 segundos virtuais) ---
    # A coreografia de keep() agora vive no bloco `main` da linguagem
    # (MainInterpreter); o loop abaixo apenas roteiriza eventos do "mundo
    # externo" (sensores ambientais) antes de cada tick.
    for seg in range(1, 13):
        if seg == 3:
            engine.fxp.human_attention = 15.0  # queda de atenção
        elif seg == 4:
            engine.fxp.cpu_power = 420.0  # alta potência -> aquece
        elif seg == 5:
            engine.fxp.solar_generation = 0.0  # painel solar para de gerar
            # Pico térmico DENTRO do horizon de TradingEspeculativo (7s),
            # tornando o cenário BDD Caso 2 (subversão térmica) alcançável.
            engine.fxp.cpu_temperature = 90.0
        elif seg == 6:
            engine.fxp.human_attention = 90.0  # atenção recupera
            if isinstance(engine.forms.get("PensarLivre"), EquilibriumForm):
                # Adiciona condição para voltar a nonequilibrium
                engine.forms["PensarLivre"].add_review_condition(
                    "attention",
                    ">",
                    80.0,
                    [{"action": "reclassify_as_nonequilibrium"}],
                )
        elif seg == 10:
            # Aumenta temperatura para disparar condição no ServidorCritico
            engine.fxp.cpu_temperature = 75.0
        elif seg == 11:
            engine.fxp.cpu_temperature = 90.0
            engine.fxp.cpu_power = 450.0

        main_block.run_due()  # executa os blocos `every` vencidos do main
        engine.tick()
        await asyncio.sleep(0.1)

    print("\n" + "=" * 50)
    print("Fim da simulação.")
    print(f"Formas ativas restantes: {list(engine.forms.keys())}")
    if engine.forms:
        for nome, forma in engine.forms.items():
            print(f"  - {nome}: {forma.value} (tipo: {type(forma).__name__})")
    print(f"Temperatura final da CPU: {engine.fxp.cpu_temperature:.2f}°C")
    print(f"Potência final da CPU: {engine.fxp.cpu_power:.2f}W")
    integro = Caderno.verify_chain()
    estado = "ÍNTEGRO" if integro else "CORROMPIDO"
    print(
        f"Integridade do Caderno (cadeia SHA-256): {estado} "
        f"— cabeça: {Caderno.chain_head()[:16]}…"
    )
    caminho = Caderno.export_jsonl("caderno_log.jsonl")
    print(f"Log do Caderno exportado para: {caminho}")


if __name__ == "__main__":
    asyncio.run(main())
