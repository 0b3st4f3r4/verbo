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

Alinhamentos da Etapa 1 (suíte em tests/ — cf. docs/STAGE-1-REPORT.md):
- FXP e tick_seconds injetáveis (simulador determinístico / mock em processo).
- Fins de forma tipificados no Caderno (FORMAL §6): dissolve_rule,
  dissolve_horizon, collapse_maintenance, dissolve_subvert; além de
  review_short_circuit, review_after_dissolution, actor_rejected_value,
  persistencia, transicao, keep_forma_inexistente, reclassify_sem_deadline.
- Persistência `.vl` canônica com SHA-256 na reclassificação (FORMAL §4.1).
- Horizon ABSOLUTO preservado nas reclassificações (FORMAL §4.1).
- Manutenção implícita enquanto houver regra de revisão ativa (FORMAL §4.1).
- Partilha igual da potência global P/N no vazamento (FORMAL §4.2).
- Contadores de retenção por forma (proxy de heap — orçamentos no ADR-001).

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
import os
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
            "Fan": {
                "min": 0,
                "max": 255,
                "safety_limit": 200,
                "current": 100,
                "apply": self._actor_fan_speed,
                "description": "Velocidade da ventoinha (PWM)",
            },
            "StatusLed": {
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
            Caderno.event(
                "actor_unknown",
                f"Ator '{actor_name}' não registrado no FXP.",
                ator=actor_name,
            )
            print(f"{RED}[FXP] Ator '{actor_name}' não encontrado!{RESET}")
            return False

        actor = self.actors[actor_name]
        # Validação de limites (se definidos) — limites INCLUSIVOS (FORMAL §4.3);
        # comando fora dos limites é rejeitado sem envio e registrado como
        # `actor_rejected_value`.
        if actor["min"] is not None and value < actor["min"]:
            Caderno.event(
                "actor_rejected_value",
                f"Comando a '{actor_name}' rejeitado sem envio: valor {value} "
                f"abaixo do mínimo.",
                ator=actor_name, valor=value, limite="min",
                limite_valor=actor["min"],
            )
            print(
                f"{YELLOW}[FXP] Valor {value} abaixo do mínimo para {actor_name}.{RESET}"
            )
            return False
        if actor["max"] is not None and value > actor["max"]:
            Caderno.event(
                "actor_rejected_value",
                f"Comando a '{actor_name}' rejeitado sem envio: valor {value} "
                f"acima do máximo.",
                ator=actor_name, valor=value, limite="max",
                limite_valor=actor["max"],
            )
            print(
                f"{YELLOW}[FXP] Valor {value} acima do máximo para {actor_name}.{RESET}"
            )
            return False
        if actor["safety_limit"] is not None and value > actor["safety_limit"]:
            Caderno.event(
                "actor_rejected_value",
                f"Comando a '{actor_name}' rejeitado sem envio: valor {value} "
                f"excede o safety_limit.",
                ator=actor_name, valor=value, limite="safety_limit",
                limite_valor=actor["safety_limit"],
            )
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
    def reset(cls):
        """Isolamento entre execuções de teste: zera eventos e a cadeia."""
        cls._events = []
        cls._chain_head = "0" * 64

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
    def event(cls, kind: str, msg: str, **extra) -> dict:
        """Registro genérico com kind explícito (kinds canônicos da FORMAL §6:
        dissolve_rule, dissolve_horizon, collapse_maintenance, dissolve_subvert,
        review_short_circuit, review_after_dissolution, actor_rejected_value,
        persistencia, transicao, ...)."""
        entry = cls._record(kind, msg, **extra)
        print(f"{WHITE}[CADERNO - {kind}]{RESET} {msg}")
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
    def info(msg: str, **extra):
        Caderno._record("INFO", msg, **extra)
        print(f"{GREEN}[CADERNO - INFO]{RESET} {msg}")

    @staticmethod
    def warn(msg: str, **extra):
        Caderno._record("ASSESSMENT", msg, **extra)
        print(f"{YELLOW}[CADERNO - AVALIAÇÃO]{RESET} {msg}")

    @staticmethod
    def alert(msg: str, **extra):
        Caderno._record("ALERT", msg, **extra)
        print(f"{RED}[CADERNO - REVISÃO COGNITIVA/ALERT]{RESET} {msg}")

    @staticmethod
    def colapso(msg: str, **extra):
        Caderno._record("COLLAPSE", msg, **extra)
        print(f"{RED}{BOLD}[CADERNO - COLAPSO TERMODINÂMICO]{RESET} {msg}")

    @staticmethod
    def art(msg: str, **extra):
        Caderno._record("SUBVERSION", msg, **extra)
        print(f"{MAGENTA}{BOLD}[CADERNO - SUBVERSÃO POÉTICA]{RESET} {msg}")

    @staticmethod
    def leak(form_name: str, power_watts: float, duration_seconds: float):
        joules = power_watts * duration_seconds
        Caderno._record(
            "LEAK",
            f"Forma '{form_name}' dissipou {joules:.2f} Joules "
            f"({power_watts:.2f} W por {duration_seconds:.2f}s)",
            forma=form_name,
            watts=power_watts,
            segundos=duration_seconds,
            joules=round(joules, 2),
        )
        print(
            f"{CYAN}[CADERNO - LEAK]{RESET} Forma '{form_name}' dissipou {joules:.2f} Joules ({power_watts:.2f} W por {duration_seconds:.2f}s)"
        )

    @staticmethod
    def sensor_read(sensor_name: str, value: float):
        Caderno._record(
            "SENSOR_READ",
            f"Sensor '{sensor_name}' = {value}",
            sensor=sensor_name,
            valor=value,
        )
        print(f"{WHITE}[CADERNO - SENSOR_READ]{RESET} Sensor '{sensor_name}' = {value}")

    @staticmethod
    def actuator_action(actor_name: str, value, success: bool):
        status = "sucesso" if success else "falha"
        Caderno._record(
            "ACTUATION",
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
        source_path: str | None,
        classification: str,
        current_time: float,
        conjugation: str = "event",
    ):
        self.name = name
        self.value = value
        self.horizon = horizon
        self.conjugation = conjugation
        self.creation_time = current_time  # tempo virtual de criação
        # `horizon` é ABSOLUTO (FORMAL §4.1): reclassificações não o renovam —
        # mantém-se o creation_time original nas reclassificações.
        self.currency = currency
        self.source_path = source_path  # nome simbólico do sensor principal (ou None)
        self.classification_currency = classification
        self.review_conditions: list[dict] = []
        # deadline DECLARADO alguma vez pela forma — sobrevive a reclassificações
        # e habilita reclassify_as_nonequilibrium posterior (FORMAL §3, nota)
        self.declared_maintenance_deadline: float | None = None
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
        source_path: str | None,
        current_time: float,
    ):
        super().__init__(
            name, value, horizon, "CpuCycles", source_path, "Transiente",
            current_time, conjugation="event",
        )


class EquilibriumForm(Form):
    """Conjugação 'equilibrium': persistente, sem manutenção, com custo em bytes."""

    def __init__(
        self,
        name: str,
        value,
        horizon: float,
        source_path: str | None,
        cost_bytes: int | None,
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
            conjugation="equilibrium",
        )
        self.cost_bytes = cost_bytes  # None -> tamanho real gravado (FORMAL §4.1)


class NonequilibriumForm(Form):
    """Conjugação 'nonequilibrium': laborativa, requer keep() contínuo."""

    def __init__(
        self,
        name: str,
        value,
        horizon: float,
        source_path: str | None,
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
            conjugation="nonequilibrium",
        )
        self.maintenance_deadline = maintenance_deadline
        self.declared_maintenance_deadline = maintenance_deadline
        self.last_maintenance = current_time
        self.exchange_mode = exchange_mode

    def keep(self, current_time: float):
        self.last_maintenance = current_time

    def check_maintenance_timeout(self, current_time: float) -> bool:
        return (current_time - self.last_maintenance) > self.maintenance_deadline


# ==============================================================================
# 4. RUNTIME ENGINE: O GOVERNO DO MOVIMENTO
# ==============================================================================
# Valor poético canônico da subversão (FORMAL §4.5)
VALOR_POETICO_CANONICO = (
    "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"
)

# Orçamentos de retenção por conjugação (AGENTS.md §1.3, reancorados no
# docs/ADR-001-linguagem-nucleo.md): o contador do runtime é um PROXY
# determinístico (estruturas de livro-razão), não medição de heap real —
# a medição física fica para a Etapa 5 (PLAN §5.1).
ORCAMENTO_RETENCAO = {"event": 256, "equilibrium": 1024, "nonequilibrium": 512}


def _fmt_num(x: float) -> str:
    """Formata número para o `.vl` canônico: inteiro sem ponto, decimal puro."""
    if isinstance(x, float) and x.is_integer():
        return str(int(x))
    return repr(x)


def _fmt_string_literal(valor) -> str:
    """Serializa o `value` como expressão canônica (string entre aspas com
    escapes; número puro; identificador)."""
    if isinstance(valor, str):
        escapado = (
            valor.replace("\\", "\\\\").replace('"', '\\"')
            .replace("\n", "\\n").replace("\t", "\\t")
        )
        return f'"{escapado}"'
    if isinstance(valor, (int, float)):
        return _fmt_num(float(valor))
    return str(valor)


def form_to_vl(form: "Form") -> str:
    """Serializa a forma em texto `.vl` canônico reparseável (FORMAL §4.1):
    conjugação, value e horizon nesta ordem, depois os opcionais aplicáveis.
    Sem vírgula final (a EBNF não a permite)."""
    linhas: list[str] = [f"{form.conjugation} {form.name} {{"]
    linhas.append(f"    value: {_fmt_string_literal(form.value)},")
    extras: list[str] = []
    if form.source_path:
        extras.append(f"source_path: {_fmt_string_literal(form.source_path)}")
    if form.conjugation == "nonequilibrium":
        extras.append(
            f"maintenance_deadline: {_fmt_num(form.maintenance_deadline)}s"
        )
        extras.append(
            f"exchange_mode: {_fmt_string_literal(getattr(form, 'exchange_mode', 'cooperation'))}"
        )
    if form.conjugation == "equilibrium" and getattr(form, "cost_bytes", None) is not None:
        extras.append(f"cost_bytes: {int(form.cost_bytes)}")
    if getattr(form, "currency", None):
        padrao = {"event": "CpuCycles", "equilibrium": "DiskBytes",
                  "nonequilibrium": "PowerWatts"}[form.conjugation]
        if form.currency != padrao:
            extras.append(f"currency: {_fmt_string_literal(form.currency)}")
    if getattr(form, "classification_currency", None):
        extras.append(
            f"classification: {_fmt_string_literal(form.classification_currency)}"
        )
    if extras:
        linhas.append(f"    horizon: {_fmt_num(form.horizon)}s,")
        linhas.extend(f"    {extra}," for extra in extras[:-1])
        linhas.append(f"    {extras[-1]}")
    else:
        linhas.append(f"    horizon: {_fmt_num(form.horizon)}s")
    linhas.append("}")
    return "\n".join(linhas) + "\n"


class VerboLangEngine:
    """Loop de processamento central: consulta FXP, avalia condições, atua e audita.

    Injeções da suíte de teste (Etapa 1): `fxp` (simulador determinístico ou
    mock em processo) e `tick_seconds` (relógio virtual — 1 tick ≈ 1 s virtual
    em produção; em teste o runtime avança instantaneamente, FORMAL §4.2).
    """

    def __init__(
        self,
        fxp: "FXP | None" = None,
        tick_seconds: float = 1.0,
        persistence_dir: str = "persistence",
    ):
        self.fxp = fxp if fxp is not None else FXP()
        self.forms: dict[str, Form] = {}
        # Contadores de retenção do runtime (proxy de heap por forma):
        self.retained_bytes: dict[str, int] = {}
        self.labor_registry: dict[str, int] = {}  # estruturas de trabalho (NEQ)
        self.clock = 0
        self.sim_time = 0.0  # relógio virtual em segundos
        self.tick_seconds = tick_seconds
        self.persistence_dir = persistence_dir

    # ------------------------------------------------------------------
    # Livro-razão de retenção por forma (contadores do runtime)
    # ------------------------------------------------------------------
    @staticmethod
    def _estimate_bytes(form: Form) -> int:
        base = {"event": 96, "equilibrium": 128, "nonequilibrium": 160}[
            form.conjugation
        ]
        valor = len(str(form.value).encode("utf-8"))
        regras = 32 * len(form.review_conditions)
        return base + valor + regras

    def _bind(self, form: Form):
        """Registra a forma ativa e recalcula os contadores de retenção."""
        self.forms[form.name] = form
        estimativa = self._estimate_bytes(form)
        self.retained_bytes[form.name] = estimativa
        if isinstance(form, NonequilibriumForm):
            # estruturas de trabalho laborativo (prazo, último keep)
            self.labor_registry[form.name] = estimativa + 24
        else:
            self.labor_registry.pop(form.name, None)

    def _unbind(self, name: str):
        self.forms.pop(name, None)
        self.retained_bytes.pop(name, None)
        self.labor_registry.pop(name, None)

    def register_form(self, form: Form):
        self._bind(form)
        Caderno.info(f"Forma '{form.name}' conjugada no sistema.")

    def dissolve_form(self, name: str, fim: str = "dissolve_rule"):
        """Dissolução com fim tipificado (FORMAL §6): dissolve_rule,
        dissolve_horizon, collapse_maintenance, dissolve_subvert.
        Recursos da forma são liberados imediatamente (contadores -> 0)."""
        if name in self.forms:
            self.forms[name].is_dissolved = True
            Caderno.event(fim, f"Forma '{name}' dissolvida ({fim}).", forma=name)
            Caderno.info(
                f"{WHITE}ALÍVIO TERMODINÂMICO{RESET} -> Forma '{name}' dissolvida."
            )
            self._unbind(name)

    def subvert_form(self, name: str):
        """FORMAL §4.5: substitui o valor pelo poético canônico e marca para
        dissolução no MESMO tick. Não há efeito físico aqui: reações do mundo
        (resfriamento etc.) são roteirizadas pelo FXP/cenário, não pelo runtime."""
        if name in self.forms:
            form = self.forms[name]
            form.value = VALOR_POETICO_CANONICO
            Caderno.art(
                f"Operador subvert() invocado na forma '{name}'! Acumulação abortada.",
                forma=name,
            )
            Caderno.event(
                "subvert_applied",
                f"Novo valor de '{name}': '{form.value}'",
                forma=name, novo_valor=form.value,
            )

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

    def persistir(self, form: "EquilibriumForm") -> tuple[str, str]:
        """Grava a forma como `.vl` canônico reparseável (FORMAL §4.1) e
        registra caminho + SHA-256 do conteúdo no Caderno."""
        os.makedirs(self.persistence_dir, exist_ok=True)
        conteudo = form_to_vl(form)
        dados = conteudo.encode("utf-8")
        caminho = os.path.join(self.persistence_dir, f"{form.name}.vl")
        with open(caminho, "w", encoding="utf-8") as f:
            f.write(conteudo)
        sha256 = hashlib.sha256(dados).hexdigest()
        Caderno.event(
            "persistence",
            f"Forma '{form.name}' persistida como `.vl` canônico.",
            forma=form.name, caminho=caminho, sha256=sha256, bytes=len(dados),
        )
        if getattr(form, "cost_bytes", None) is None:
            # cost_bytes ausente passa a valer o tamanho real gravado (§4.1)
            form.cost_bytes = len(dados)
        return caminho, sha256

    def execute_actions(self, form: Form, actions: list[dict]) -> bool:
        """
        Executa a lista de ações associadas a uma condição disparada, na ordem
        declarada (FORMAL §4.2/§4.5). Retorna True se a forma deixou de existir
        na conjugação anterior (dissolvida/reclassificada) — as regras
        seguintes da mesma review não são avaliadas naquele tick
        (`review_short_circuit`, registrado pelo tick).
        """
        if form.name not in self.forms or self.forms[form.name].is_dissolved:
            # Ação de revisão sobre forma já dissolvida no mesmo tick é
            # ignorada, com registro (FORMAL §4.1).
            Caderno.event(
                "review_after_dissolution",
                f"Ação de revisão sobre '{form.name}' ignorada: forma já "
                f"dissolvida neste tick.",
                forma=form.name,
            )
            return True
        doomed = False  # subvert marca a forma para dissolução no mesmo tick
        for act in actions:
            action_type = act.get("action")
            if action_type == "dissolve":
                self.dissolve_form(form.name, fim="dissolve_rule")
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
                # Persiste em disco (`.vl` canônico + SHA-256, FORMAL §4.1) e
                # converte para EquilibriumForm. O horizon é ABSOLUTO (§4.1):
                # o creation_time original é preservado na reclassificação.
                self.fxp.disk_bytes_used += 1024  # simula escrita no suporte
                new_form = EquilibriumForm(
                    name=form.name,
                    value=form.value,
                    horizon=form.horizon,
                    source_path=form.source_path,  # preserva a vinculação
                    cost_bytes=getattr(form, "cost_bytes", None),
                    current_time=form.creation_time,  # horizon absoluto
                )
                new_form.declared_maintenance_deadline = (
                    form.declared_maintenance_deadline
                )
                new_form.review_conditions = form.review_conditions.copy()
                Caderno.event(
                    "transition",
                    f"Forma '{form.name}' reclassificada para 'equilibrium' "
                    f"(persistida).",
                    forma=form.name, de=form.conjugation, para="equilibrium",
                )
                self.persistir(new_form)
                self._bind(new_form)
                return True
            elif action_type == "reclassify_as_nonequilibrium":
                # Só é legal se a forma já DECLAROU um maintenance_deadline
                # alguma vez (FORMAL §3): sem deadline declarado é erro de
                # runtime registrado no Caderno — a forma permanece como estava.
                deadline = form.declared_maintenance_deadline
                if deadline is None:
                    Caderno.event(
                        "reclassify_no_deadline",
                        f"reclassify_as_nonequilibrium recusado para "
                        f"'{form.name}': sem maintenance_deadline declarado "
                        f"(FORMAL §3). A forma permanece como estava.",
                        forma=form.name,
                    )
                    return True
                modo = getattr(form, "exchange_mode", "cooperation")
                new_form = NonequilibriumForm(
                    name=form.name,
                    value=form.value,
                    horizon=form.horizon,
                    source_path=form.source_path,
                    maintenance_deadline=deadline,
                    exchange_mode=modo,
                    current_time=form.creation_time,  # horizon absoluto
                )
                new_form.review_conditions = form.review_conditions.copy()
                Caderno.event(
                    "transition",
                    f"Forma '{form.name}' reclassificada para "
                    f"'nonequilibrium' (trabalho ativo).",
                    forma=form.name, de=form.conjugation,
                    para="nonequilibrium",
                )
                self._bind(new_form)
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
            self.dissolve_form(form.name, fim="dissolve_subvert")
            return True
        return False

    def tick(self):
        """Avança um segundo virtual e processa todas as formas."""
        self.clock += 1
        self.sim_time += self.tick_seconds
        print(f"\n{BOLD}--- SEGUNDO {self.clock} ---{RESET}")

        # Atualiza o estado do hardware uma única vez por tick
        self.fxp.update_hardware_state()

        # Itera sobre uma cópia das chaves para permitir remoção segura
        nomes_ativos = list(self.forms.keys())
        total_ativas = len(nomes_ativos)
        for name in nomes_ativos:
            if name not in self.forms:
                continue

            form = self.forms[name]

            # 1. Auditoria de vazamento energético — a potência lida no tick
            #    (cpu_power, global) é repartida IGUALMENTE entre as formas
            #    ativas: P/N × duração do tick (docs/FORMAL.md §4.2). A
            #    conversão para a `currency` da forma usa fator 1.0 neste
            #    protótipo (método de atribuição refinado na Etapa 4).
            power_consumption = (
                self.fxp.cpu_power / total_ativas if total_ativas else 0.0
            )
            Caderno.leak(form.name, power_consumption, self.tick_seconds)

            # 2. Leitura do sensor associado à forma (source_path) e registro;
            #    formas sem source_path não geram leitura.
            if form.source_path:
                sensor_value = self.fxp.read_sensor(form.source_path)
                if sensor_value is not None:
                    Caderno.sensor_read(form.source_path, sensor_value)

            # 3. Avaliação das condições de revisão
            #    Cada condição pode usar um sensor diferente; as regras são
            #    avaliadas na ORDEM DECLARADA, antes da verificação de prazos
            #    (docs/FORMAL.md §4.2).
            condition_triggered = False
            for indice, cond in enumerate(form.review_conditions):
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
                        restantes = len(form.review_conditions) - indice - 1
                        if restantes > 0:
                            # review_short_circuit: regras seguintes da mesma
                            # review não são avaliadas naquele tick — sem
                            # revogar atuações já despachadas (§4.2/§4.5)
                            Caderno.event(
                                "review_short_circuit",
                                f"'{form.name}': {restantes} regra(s) seguinte(s) "
                                f"não avaliada(s) neste tick.",
                                forma=form.name, regras_restantes=restantes,
                            )
                        break  # sai do loop de condições
            if condition_triggered or name not in self.forms:
                continue

            # 4. Verificação de manutenção (apenas nonequilibrium).
            #    Manutenção IMPLÍCITA do runtime (docs/FORMAL.md §4.1): a cada
            #    tick, enquanto a forma tiver ao menos uma regra de revisão
            #    ativa. Sem regra ativa e sem keep(), colapsa no primeiro
            #    vencimento do maintenance_deadline.
            if isinstance(form, NonequilibriumForm):
                if form.review_conditions:
                    form.keep(self.sim_time)
                elif form.check_maintenance_timeout(self.sim_time):
                    Caderno.colapso(
                        f"Prazo de manutenção de '{form.name}' expirou! "
                        f"(sem keep() por {form.maintenance_deadline}s)",
                        forma=form.name,
                    )
                    self.dissolve_form(form.name, fim="collapse_maintenance")
                    continue

            # 5. Verificação de horizonte — age apenas se a forma seguir
            #    ativa ao final do passo (docs/FORMAL.md §4.2)
            if form.check_horizon(self.sim_time):
                Caderno.warn(
                    f"Horizonte de validade de '{form.name}' esgotou-se. Dissolvendo.",
                    forma=form.name,
                )
                self.dissolve_form(form.name, fim="dissolve_horizon")


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
            if form is None:
                # Cláusula de erro: keep de forma inexistente/dissolvida —
                # registrado no Caderno, sem interromper o runtime.
                Caderno.event(
                    "keep_unknown_form",
                    f"keep('{st['form']}'): forma inexistente ou já dissolvida.",
                    forma=st["form"],
                )
            elif isinstance(form, NonequilibriumForm):
                form.keep(self.engine.sim_time)
            else:
                Caderno.event(
                    "keep_ignored",
                    f"keep('{st['form']}'): conjugação {form.conjugation} não "
                    f"exige manutenção.",
                    forma=st["form"],
                )
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
    # Exemplo 4 — canônico: every 4s { keep(ImportantTask) } — para as
    # três formas conjuradas abaixo):
    #   every 1s  { keep(FreeThinking), keep(SpeculativeTrading), keep(ServidorCritico) }
    #   every 10s { act(StatusLed, "green") }
    main_block = MainInterpreter(engine)
    main_block.add_every(
        1.0,
        [
            {"statement": "keep", "form": "FreeThinking"},
            {"statement": "keep", "form": "SpeculativeTrading"},
            {"statement": "keep", "form": "ServidorCritico"},
        ],
    )
    main_block.add_every(
        10.0, [{"statement": "act", "actor": "StatusLed", "value": "green"}]
    )

    # --- CONJURAÇÃO 1: FreeThinking (nonequilibrium) ---
    # Sensor principal: attention
    # Condição adicional: usa o mesmo sensor, mas poderia usar outro.
    pensar_livre = NonequilibriumForm(
        name="FreeThinking",
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

    # --- CONJURAÇÃO 2: SpeculativeTrading (nonequilibrium) ---
    # Sensor principal: cpu_temp
    # Condição adicional: usa cpu_temp e também ação com ator.
    trading_predatorio = NonequilibriumForm(
        name="SpeculativeTrading",
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
        "cpu_temp", ">", 70.0, [{"action": "act", "actor": "Fan", "value": 200}]
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
        f"Atuação inicial no ator 'Fan': {engine.fxp.act('Fan', 150)}"
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
            # Pico térmico DENTRO do horizon de SpeculativeTrading (7s),
            # tornando o cenário BDD Caso 2 (subversão térmica) alcançável.
            engine.fxp.cpu_temperature = 90.0
        elif seg == 6:
            engine.fxp.human_attention = 90.0  # atenção recupera
            # O mundo reage à subversão do tick anterior (potência capada):
            # efeito físico roteirizado pelo cenário — o runtime não faz
            # física por conta própria (FORMAL §4.5).
            engine.fxp.cpu_power = 15.0
            if isinstance(engine.forms.get("FreeThinking"), EquilibriumForm):
                # Adiciona condição para voltar a nonequilibrium
                engine.forms["FreeThinking"].add_review_condition(
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
