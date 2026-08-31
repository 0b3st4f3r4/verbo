# -*- coding: utf-8 -*-
"""Testes de Comandos de Atores (PLAN §1.2) — FORMAL §4.3.

Simular envio de `act` e validar que a mensagem FXP é serializada e entregue
ao ator correto (mock); limites inclusivos; rejeição sem envio registrada
como `actor_rejected_value`; fallback como política do registro do FXP.
"""

from __future__ import annotations

import pytest

from fxp_sim import ir, loader


def _carregar(engine, *declaracoes, main=None):
    return loader.carregar(engine, ir.programa(*declaracoes, main=main))


# ----------------------------------------------------------------------
# Serialização e entrega (fronteira mock em processo)
# ----------------------------------------------------------------------
def test_act_e_serializado_e_entregue_ao_ator_correto(engine, cad, sim):
    _carregar(
        engine,
        ir.forma("Servidor", "nonequilibrium", "critico", "30s",
                 source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Servidor", ir.regra("cpu_temp", ">", 70, "°C",
                                       ir.act_("Ventoinha", 200))),
    )
    sim.set_sensor("cpu_temp", 75.0)
    engine.tick()
    # mensagem serializada no outbox do FXP
    msg = [m for m in sim.outbox if m["op"] == "act"
           and m["ator"] == "Ventoinha" and m["valor"] == 200]
    assert msg, "mensagem FXP `act` não serializada"
    assert msg[0]["tick"] == engine.clock  # tick de despacho registrado
    # entrega ao ator correto
    assert any(e["ator"] == "Ventoinha" and e["valor"] == 200
               for e in sim.entregues)
    assert sim.atores["Ventoinha"].atual == 200
    assert cad.tem("ATUACAO", ator="Ventoinha", valor=200, sucesso=True)


def test_ator_inexistente_rejeitado_com_registro(engine, cad, sim):
    """Cláusula de erro: ator fora do registro do FXP."""
    _carregar(
        engine,
        ir.forma("Tarefa", "event", "x", "10s"),
        ir.review("Tarefa", ir.regra("cpu_temp", ">", 10, "°C",
                                     ir.act_("AtorFantasma", 10))),
    )
    sim.set_sensor("cpu_temp", 30.0)
    engine.tick()
    assert cad.tem("ator_inexistente", ator="AtorFantasma")
    assert cad.tem("ATUACAO", ator="AtorFantasma", sucesso=False)
    assert not any(e["ator"] == "AtorFantasma" for e in sim.entregues)


# ----------------------------------------------------------------------
# Limites e segurança (FORMAL §4.3, registro mínimo §6)
# ----------------------------------------------------------------------
def test_valor_abaixo_do_minimo_rejeitado_sem_envio(engine, cad, sim):
    _carregar(
        engine,
        ir.forma("T", "event", "x", "10s"),
        ir.review("T", ir.regra("cpu_temp", ">", 10, "°C",
                                ir.act_("CpuPowerCap", 5))),
    )
    sim.set_sensor("cpu_temp", 30.0)
    engine.tick()
    evento = cad.buscar("actor_rejected_value", ator="CpuPowerCap")
    assert evento and evento[0]["limite"] == "min" and evento[0]["limite_valor"] == 10
    assert not any(e["ator"] == "CpuPowerCap" for e in sim.entregues)


def test_valor_acima_do_safety_limit_rejeitado(engine, cad, sim):
    _carregar(
        engine,
        ir.forma("T", "event", "x", "10s"),
        ir.review("T", ir.regra("cpu_temp", ">", 10, "°C",
                                ir.act_("Ventoinha", 250))),
    )
    sim.set_sensor("cpu_temp", 30.0)
    engine.tick()
    evento = cad.buscar("actor_rejected_value", ator="Ventoinha")
    assert evento and evento[0]["limite"] == "safety_limit" \
        and evento[0]["limite_valor"] == 200
    assert not any(e["ator"] == "Ventoinha" for e in sim.entregues)


def test_limites_sao_inclusivos(engine, cad, sim):
    """FORMAL §4.3: valor IGUAL ao limite é aceito (ex.: safety_limit)."""
    assert sim.act("Ventoinha", 200) is True      # == safety_limit
    assert sim.act("Ventoinha", 0) is True        # == min
    assert sim.act("CpuPowerCap", 200) is True    # == safety_limit
    assert sim.act("CpuPowerCap", 10) is True     # == min
    assert not cad.tem("actor_rejected_value")


@pytest.mark.parametrize("ator,valor", [("Ventoinha", 256), ("CpuPowerCap", 251),
                                        ("CpuPowerCap", 9)])
def test_valores_fora_de_limite_rejeitados(engine, cad, sim, ator, valor):
    assert sim.act(ator, valor) is False
    assert cad.tem("actor_rejected_value", ator=ator)


def test_rejeicao_nao_dissolve_a_forma(engine, cad, sim):
    """FORMAL §4.3: a forma não é dissolvida pela rejeição do comando."""
    _carregar(
        engine,
        ir.forma("Insistente", "nonequilibrium", "obs", "30s",
                 source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Insistente", ir.regra("cpu_temp", ">", 70, "°C",
                                         ir.act_("CpuPowerCap", 5))),
    )
    sim.set_sensor("cpu_temp", 80.0)
    engine.tick()
    assert cad.tem("actor_rejected_value")
    assert "Insistente" in engine.forms  # forma sobrevive à rejeição
    assert not cad.tem("dissolve_rule")


# ----------------------------------------------------------------------
# Fallback: política do REGISTRO do FXP (FORMAL §4.3; BDD Caso 3)
# ----------------------------------------------------------------------
def _preparar_fallback(sim):
    sim.registrar_ator("VentoinhaReserva", "ventoinha alternativa (extensão)",
                       minimo=0, maximo=255, safety_limit=200)
    sim.definir_fallback("Ventoinha", "VentoinhaReserva")


def test_fallback_executado_quando_primario_nao_responde(engine, cad, sim):
    _preparar_fallback(sim)
    _carregar(
        engine,
        ir.forma("Servidor", "nonequilibrium", "critico", "3600s",
                 source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Servidor", ir.regra("cpu_temp", ">", 70, "°C",
                                       ir.act_("Ventoinha", 200))),
    )
    sim.falhar_ator("Ventoinha")
    sim.set_sensor("cpu_temp", 75.0)
    engine.tick()
    # tentativa primária registrada como falha + heartbeat indisponível
    assert cad.tem("ATUACAO", ator="Ventoinha", sucesso=False)
    assert cad.tem("ator_indisponivel", ator="Ventoinha")
    # fallback executado e entregue
    assert cad.tem("fallback_executado", primario="Ventoinha",
                   alternativo="VentoinhaReserva")
    assert sim.atores["VentoinhaReserva"].atual == 200
    assert sim.atores["Ventoinha"].atual != 200


def test_fallback_esgotado_registra_alerta(engine, cad, sim):
    _preparar_fallback(sim)
    sim.falhar_ator("Ventoinha")
    sim.falhar_ator("VentoinhaReserva")
    ok = sim.act("Ventoinha", 200)
    assert ok is False
    assert cad.tem("ALERTA", motivo="fallback_esgotado", ator="Ventoinha")
    assert not sim.entregues


# ----------------------------------------------------------------------
# Fronteira mock pura (sem modelo físico) — PLAN §1.3 b
# ----------------------------------------------------------------------
def test_mockfxp_serializa_e_valida_sem_modelo_fisico(vbl, cad):
    from fxp_sim.mocks import MockFXP

    mock = MockFXP(caderno=vbl.Caderno)
    mock.registrar_sensor("cpu_temp", 55.0)
    mock.registrar_ator("Ventoinha", minimo=0, maximo=255, safety=200)
    assert mock.read_sensor("cpu_temp") == 55.0
    assert mock.read_sensor("solar_panel") is None  # ausente -> None, nunca 0.0
    assert mock.act("Ventoinha", 200) is True
    assert mock.act("Ventoinha", 300) is False  # acima do max
    assert mock.act("LedFantasma", 1) is False  # ator inexistente
    assert [m["ator"] for m in mock.outbox] == ["Ventoinha", "Ventoinha", "LedFantasma"]
    assert [m["valor"] for m in mock.entregues] == [200]
