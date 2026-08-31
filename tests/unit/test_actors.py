# -*- coding: utf-8 -*-
"""Testes de Comandos de Atores (PLAN §1.2) — FORMAL §4.3.

Simular envio de `act` e validar que a mensagem FXP é serializada e entregue
ao ator correto (mock); limites inclusivos; rejeição sem envio registrada
como `actor_rejected_value`; fallback como política do registro do FXP.
"""

from __future__ import annotations

import pytest

from fxp_sim import ir, loader


def _load(engine, *declarations, main=None):
    return loader.load(engine, ir.program(*declarations, main=main))


# ----------------------------------------------------------------------
# Serialização e entrega (fronteira mock em processo)
# ----------------------------------------------------------------------
def test_act_is_serialized_and_delivered_to_correct_actor(engine, ledger, sim):
    _load(
        engine,
        ir.form("Servidor", "nonequilibrium", "critico", "30s",
                source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Servidor", ir.rule("cpu_temp", ">", 70, "°C",
                                      ir.act_("Ventoinha", 200))),
    )
    sim.set_sensor("cpu_temp", 75.0)
    engine.tick()
    # mensagem serializada no outbox do FXP
    msg = [m for m in sim.outbox if m["op"] == "act"
           and m["actor"] == "Ventoinha" and m["value"] == 200]
    assert msg, "mensagem FXP `act` não serializada"
    assert msg[0]["tick"] == engine.clock  # tick de despacho registrado
    # entrega ao ator correto
    assert any(e["actor"] == "Ventoinha" and e["value"] == 200
               for e in sim.delivered)
    assert sim.actors["Ventoinha"].current == 200
    assert ledger.has("ACTUATION", ator="Ventoinha", valor=200, sucesso=True)


def test_unregistered_actor_rejected_and_recorded(engine, ledger, sim):
    """Cláusula de erro: ator fora do registro do FXP."""
    _load(
        engine,
        ir.form("Tarefa", "event", "x", "10s"),
        ir.review("Tarefa", ir.rule("cpu_temp", ">", 10, "°C",
                                    ir.act_("AtorFantasma", 10))),
    )
    sim.set_sensor("cpu_temp", 30.0)
    engine.tick()
    assert ledger.has("actor_unknown", ator="AtorFantasma")
    assert ledger.has("ACTUATION", ator="AtorFantasma", sucesso=False)
    assert not any(e["actor"] == "AtorFantasma" for e in sim.delivered)


# ----------------------------------------------------------------------
# Limites e segurança (FORMAL §4.3, registro mínimo §6)
# ----------------------------------------------------------------------
def test_value_below_minimum_rejected_without_send(engine, ledger, sim):
    _load(
        engine,
        ir.form("T", "event", "x", "10s"),
        ir.review("T", ir.rule("cpu_temp", ">", 10, "°C",
                               ir.act_("CpuPowerCap", 5))),
    )
    sim.set_sensor("cpu_temp", 30.0)
    engine.tick()
    event = ledger.find("actor_rejected_value", ator="CpuPowerCap")
    assert event and event[0]["limite"] == "min" and event[0]["limite_valor"] == 10
    assert not any(e["actor"] == "CpuPowerCap" for e in sim.delivered)


def test_value_above_safety_limit_rejected(engine, ledger, sim):
    _load(
        engine,
        ir.form("T", "event", "x", "10s"),
        ir.review("T", ir.rule("cpu_temp", ">", 10, "°C",
                               ir.act_("Ventoinha", 250))),
    )
    sim.set_sensor("cpu_temp", 30.0)
    engine.tick()
    event = ledger.find("actor_rejected_value", ator="Ventoinha")
    assert event and event[0]["limite"] == "safety_limit" \
        and event[0]["limite_valor"] == 200
    assert not any(e["actor"] == "Ventoinha" for e in sim.delivered)


def test_limits_are_inclusive(engine, ledger, sim):
    """FORMAL §4.3: valor IGUAL ao limite é aceito (ex.: safety_limit)."""
    assert sim.act("Ventoinha", 200) is True      # == safety_limit
    assert sim.act("Ventoinha", 0) is True        # == min
    assert sim.act("CpuPowerCap", 200) is True    # == safety_limit
    assert sim.act("CpuPowerCap", 10) is True     # == min
    assert not ledger.has("actor_rejected_value")


@pytest.mark.parametrize("actor,value", [("Ventoinha", 256), ("CpuPowerCap", 251),
                                         ("CpuPowerCap", 9)])
def test_values_out_of_limit_rejected(engine, ledger, sim, actor, value):
    assert sim.act(actor, value) is False
    assert ledger.has("actor_rejected_value", ator=actor)


def test_rejection_does_not_dissolve_the_form(engine, ledger, sim):
    """FORMAL §4.3: a forma não é dissolvida pela rejeição do comando."""
    _load(
        engine,
        ir.form("Insistente", "nonequilibrium", "obs", "30s",
                source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Insistente", ir.rule("cpu_temp", ">", 70, "°C",
                                        ir.act_("CpuPowerCap", 5))),
    )
    sim.set_sensor("cpu_temp", 80.0)
    engine.tick()
    assert ledger.has("actor_rejected_value")
    assert "Insistente" in engine.forms  # forma sobrevive à rejeição
    assert not ledger.has("dissolve_rule")


# ----------------------------------------------------------------------
# Fallback: política do REGISTRO do FXP (FORMAL §4.3; BDD Caso 3)
# ----------------------------------------------------------------------
def _prepare_fallback(sim):
    sim.register_actor("VentoinhaReserva", "ventoinha alternativa (extensão)",
                       min_value=0, max_value=255, safety_limit=200)
    sim.define_fallback("Ventoinha", "VentoinhaReserva")


def test_fallback_executed_when_primary_does_not_respond(engine, ledger, sim):
    _prepare_fallback(sim)
    _load(
        engine,
        ir.form("Servidor", "nonequilibrium", "critico", "3600s",
                source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Servidor", ir.rule("cpu_temp", ">", 70, "°C",
                                      ir.act_("Ventoinha", 200))),
    )
    sim.fail_actor("Ventoinha")
    sim.set_sensor("cpu_temp", 75.0)
    engine.tick()
    # tentativa primária registrada como falha + heartbeat indisponível
    assert ledger.has("ACTUATION", ator="Ventoinha", sucesso=False)
    assert ledger.has("actor_unavailable", ator="Ventoinha")
    # fallback executado e entregue
    assert ledger.has("fallback_executed", primario="Ventoinha",
                      alternativo="VentoinhaReserva")
    assert sim.actors["VentoinhaReserva"].current == 200
    assert sim.actors["Ventoinha"].current != 200


def test_exhausted_fallback_records_alert(engine, ledger, sim):
    _prepare_fallback(sim)
    sim.fail_actor("Ventoinha")
    sim.fail_actor("VentoinhaReserva")
    ok = sim.act("Ventoinha", 200)
    assert ok is False
    assert ledger.has("ALERT", motivo="fallback_esgotado", ator="Ventoinha")
    assert not sim.delivered


# ----------------------------------------------------------------------
# Fronteira mock pura (sem modelo físico) — PLAN §1.3 b
# ----------------------------------------------------------------------
def test_mockfxp_serializes_and_validates_without_physical_model(vbl, ledger):
    from fxp_sim.mocks import MockFXP

    mock = MockFXP(ledger=vbl.Caderno)
    mock.register_sensor("cpu_temp", 55.0)
    mock.register_actor("Ventoinha", min_value=0, max_value=255, safety=200)
    assert mock.read_sensor("cpu_temp") == 55.0
    assert mock.read_sensor("solar_panel") is None  # ausente -> None, nunca 0.0
    assert mock.act("Ventoinha", 200) is True
    assert mock.act("Ventoinha", 300) is False  # acima do max
    assert mock.act("LedFantasma", 1) is False  # ator inexistente
    assert [m["actor"] for m in mock.outbox] == ["Ventoinha", "Ventoinha", "LedFantasma"]
    assert [m["value"] for m in mock.delivered] == [200]
