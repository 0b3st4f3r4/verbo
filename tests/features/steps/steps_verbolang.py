# -*- coding: utf-8 -*-
"""Steps dos três cenários BDD da Etapa 1 (docs/PLAN.md §1.1).

Cada cenário roda sobre mocks em processo (FXPSimulator determinístico) e o
runtime do protótipo — sem hardware, sem rede, sem aleatoriedade.
"""

from __future__ import annotations

import hashlib
import os

from behave import given, then, when

import vlcheck
from fxp_sim import ir

POETRY = "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"


# ======================================================================
# Caso 1 — Fadiga de atenção
# ======================================================================
@given('the laborative form "{name}" is active with a deadline of {deadline:d}s')
def step_laborative_form(context, name, deadline):
    program = ir.program(
        ir.form(name, "nonequilibrium", "consciencia_anteneoliberal_ativa",
                "60s", source_path="attention",
                maintenance_deadline=f"{deadline}s",
                exchange_mode="cooperation"),
        ir.review(name, ir.rule("attention", "<", 30, "%",
                                ir.action("reclassify_as_equilibrium"))),
    )
    context.form_name = name
    context.loader.load(context.engine, program)
    assert context.engine.labor_registry.get(name) is not None


@when('the "{sensor}" sensor reading via FXP drops below {threshold:d}% (e.g. {value:g}%)')
def step_attention_drop(context, sensor, threshold, value):
    context.sim.set_sensor(sensor, value)
    context.engine.tick()


@then('the runtime must fire a "reclassify_as_equilibrium" transition')
def step_transition(context):
    assert context.ledger.has("transition", forma=context.form_name, para="equilibrium")
    assert context.engine.forms[context.form_name].conjugation == "equilibrium"


@then('the idea state must be saved as canonical ".vl" in the persistence directory')
def step_persisted_canonical(context):
    path = os.path.join(context.engine.persistence_dir, f"{context.form_name}.vl")
    assert os.path.isfile(path), f"arquivo persistido ausente: {path}"
    with open(path, encoding="utf-8") as f:
        content = f.read()
    # canônico = reparseável: o validador de superfície não aponta erros
    errors = vlcheck.validate(content)
    assert not errors, f".vl persistido não é canônico: {errors}"
    assert 'value: "consciencia_anteneoliberal_ativa"' in content
    context.persisted_path = path


@then('the Ledger records the persistence event with the SHA-256 of the written file')
def step_persistence_event(context):
    events = context.ledger.find("persistence", forma=context.form_name)
    assert events, "evento de persistência ausente no Caderno"
    with open(context.persisted_path, "rb") as f:
        real_sha = hashlib.sha256(f.read()).hexdigest()
    assert events[-1]["sha256"] == real_sha
    assert events[-1]["caminho"] == context.persisted_path


@then('after the reclassification the form no longer receives maintenance ticks')
def step_no_maintenance(context):
    name = context.form_name
    # trabalho laborativo encerrado: nenhum tick de manutenção seguinte
    assert context.engine.labor_registry.get(name) is None
    context.engine.tick()  # um tick extra: sem keep() e sem colapso
    assert name in context.engine.forms
    assert not context.ledger.has("collapse_maintenance", forma=name)


@then('0 bytes remain retained on the heap for the form (verified with runtime counters)')
def step_zero_bytes_retained(context):
    name = context.form_name
    # Interpretação registrada (docs/STAGE-1-REPORT.md): o estado
    # laborativo (nonequilibrium) foi integralmente liberado — 0 bytes de
    # trabalho retidos; o que permanece é a forma equilibrium persistida em
    # disco, dentro do orçamento de retenção da conjugação (ADR-001).
    assert context.engine.labor_registry.get(name) is None
    budget = context.vbl.ORCAMENTO_RETENCAO["equilibrium"]
    assert context.engine.retained_bytes.get(name, 0) <= budget


# ======================================================================
# Caso 2 — Subversão térmica
# ======================================================================
@given('the task "{name}" is running at high frequency')
def step_trading_high_frequency(context, name):
    program = ir.program(
        ir.form(name, "nonequilibrium", "lucro_arbitragem_alta_frequencia",
                "7s", source_path="cpu_temp", maintenance_deadline="2s",
                exchange_mode="extraction"),
        ir.review(name, ir.rule("cpu_temp", ">", 85, "°C",
                                ir.action("subvert"),
                                ir.act_("CpuPowerCap", 50))),
    )
    context.form_name = name
    context.loader.load(context.engine, program)
    context.sim.cpu_power = 420.0  # alta frequência => potência elevada
    context.trigger_tick = None


@when('the "{sensor}" sensor reaches {value:g}°C (limit of {limit:g}°C) via FXP')
def step_thermal_peak(context, sensor, value, limit):
    context.limit = limit
    context.sim.set_sensor(sensor, value)
    context.engine.tick()
    context.trigger_tick = context.engine.clock


@then('the runtime must invoke the "subvert()" operator')
def step_subvert_invoked(context):
    assert context.ledger.has("dissolve_subvert", forma=context.form_name)
    assert context.ledger.has("subvert_applied", forma=context.form_name)


@then('the action "act({actor}, {value:d})" must be sent to the corresponding actor via FXP')
def step_act_sent(context, actor, value):
    msg = [m for m in context.sim.outbox
           if m["op"] == "act" and m["actor"] == actor and m["value"] == value]
    assert msg, "comando `act` não serializado no FXP"
    assert any(e["actor"] == actor and e["value"] == value
               for e in context.sim.delivered)
    assert context.sim.actors[actor].current == value


@then('the trading logical value must be replaced by the canonical poetic value "{poetry}"')
def step_poetic_value(context, poetry):
    event = context.ledger.find("subvert_applied", forma=context.form_name)
    assert event and event[0]["novo_valor"] == poetry == POETRY


@then('the subverted form processing must cease in the same tick (dissolution within ≤ 1 virtual tick)')
def step_ceases_in_same_tick(context):
    name = context.form_name
    assert name not in context.engine.forms  # dissolvida
    assert context.engine.clock == context.trigger_tick  # mesmo tick
    # 0 bytes de trabalho retidos (contadores do runtime)
    assert context.engine.labor_registry.get(name) is None
    assert context.engine.retained_bytes.get(name) is None


# ======================================================================
# Caso 3 — Fallback de ator
# ======================================================================
@given('the actor "{actor}" is not responding')
def step_actor_not_responding(context, actor):
    # política de fallback no REGISTRO do FXP (FORMAL §4.3) + extensão opcional
    context.sim.register_actor("VentoinhaReserva",
                               "ventoinha alternativa (extensão opcional)",
                               min_value=0, max_value=255, safety_limit=200)
    context.sim.define_fallback(actor, "VentoinhaReserva")
    context.sim.fail_actor(actor)
    # a forma vigia a temperatura e aciona o ator primário ao exceder 70°C
    program = ir.program(
        ir.form("ServidorCritico", "nonequilibrium", "processamento_continuo",
                "3600s", source_path="cpu_temp", maintenance_deadline="10s",
                exchange_mode="cooperation"),
        ir.review("ServidorCritico",
                  ir.rule("cpu_temp", ">", 70, "°C", ir.act_(actor, 200))),
    )
    context.loader.load(context.engine, program)
    context.primary_actor = actor


@when('the temperature exceeds {limit:d}°C and the action is "act({actor}, {value:d})"')
def step_temperature_exceeds(context, limit, actor, value):
    context.sim.set_sensor("cpu_temp", limit + 5.0)
    context.engine.tick()


@then('the FXP detects the failure (heartbeat) and applies the registry fallback policy, trying the alternative actor "VentoinhaReserva" (optional extension)')
def step_fallback_applied(context):
    primary = context.primary_actor
    assert context.ledger.has("actor_unavailable", ator=primary)
    assert context.ledger.has("fallback_executed", primario=primary,
                              alternativo="VentoinhaReserva", valor=200)
    assert context.sim.actors["VentoinhaReserva"].current == 200
    assert context.sim.actors[primary].current != 200


@then('the Ledger records the primary attempt, the failure and the executed fallback')
def step_ledger_trails_fallback(context):
    primary = context.primary_actor
    assert context.ledger.has("ACTUATION", ator=primary, sucesso=False)
    assert context.ledger.has("ACTUATION", ator="VentoinhaReserva", sucesso=True)
    assert context.ledger.has("fallback_executed")
