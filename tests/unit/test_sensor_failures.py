# -*- coding: utf-8 -*-
"""Testes de Falha Controlada de sensores (PLAN §1.2) — FORMAL §4.7.

(i) leitura `0.0` é válida e avaliada normalmente;
(ii) sensor ausente ou inacessível: nenhuma `when` é avaliada, o Caderno
     registra alerta e não há disparo falso — sensor ausente NUNCA é 0.0.
"""

from __future__ import annotations

from fxp_sim import ir, loader


def _load(engine, *declarations):
    return loader.load(engine, ir.program(*declarations))


def _attention_form():
    return (
        ir.form("Sentinela", "nonequilibrium", "vigia", "30s",
                source_path="attention", maintenance_deadline="3s"),
        ir.review("Sentinela",
                  ir.rule("attention", "<", 30, "%",
                          ir.action("reclassify_as_equilibrium"))),
    )


def test_zero_reading_is_valid_and_fires_rules(engine, ledger, sim):
    """0.0 é leitura física legítima: a regra `attention < 30%` DEVE disparar."""
    _load(engine, *_attention_form())
    sim.set_sensor("attention", 0.0)
    engine.tick()
    assert ledger.has("transicao", forma="Sentinela", para="equilibrium")
    readings = [e for e in ledger.find("LEITURA", sensor="attention")]
    assert readings and readings[-1]["valor"] == 0.0
    # zero NÃO é falha de I/O: nenhum alerta de sensor
    assert not ledger.has("ALERTA", motivo="sensor_nao_registrado")
    assert not ledger.has("ALERTA", motivo="sensor_inacessivel")


def test_zero_reading_is_never_an_io_failure(engine, ledger, sim):
    sim.set_sensor("cpu_temp", 0.0)
    assert sim.read_sensor("cpu_temp") == 0.0  # valor, não None
    assert not ledger.has("ALERTA")


def test_missing_sensor_evaluates_no_condition_nor_fires(engine, ledger, sim):
    """FORMAL §4.7: sensor fora do registro é falha de I/O — a condição não
    é avaliada naquele tick e não há disparo falso (o valor NÃO é 0.0)."""
    _load(
        engine,
        ir.form("Fantasma", "nonequilibrium", "obs", "30s",
                source_path="sensor_inexistente", maintenance_deadline="3s"),
        ir.review("Fantasma",
                  ir.rule("sensor_inexistente", "<", 30, "%",
                          ir.action("reclassify_as_equilibrium"))),
    )
    for _ in range(4):
        engine.tick()
    # a regra jamais avaliou: forma permanece nonequilibrium
    assert engine.forms["Fantasma"].conjugation == "nonequilibrium"
    assert not ledger.has("transicao")
    # alerta registrado a cada tick de leitura
    alerts = ledger.find("ALERTA", motivo="sensor_nao_registrado",
                         sensor="sensor_inexistente")
    assert len(alerts) >= 1


def test_missing_sensor_is_not_treated_as_zero(engine, ledger, sim):
    """Se sensor ausente valesse 0.0, a regra `attention < 30%` dispararia
    falsamente — é exatamente o que a FORMAL §4.7 proíbe."""
    _load(engine, *_attention_form())
    sim.unregister_sensor("attention")
    for _ in range(3):
        engine.tick()
    assert not ledger.has("transicao")  # nenhum disparo falso
    assert engine.forms["Sentinela"].conjugation == "nonequilibrium"


def test_registered_but_inaccessible_sensor_follows_same_rule(engine, ledger, sim):
    """FORMAL §4.7: registrado porém inacessível (falha de leitura em modo
    real) segue a mesma regra — condição não avaliada + alerta."""
    _load(engine, *_attention_form())
    sim.fail_sensor("attention")
    engine.tick()
    assert not ledger.has("transicao")
    assert ledger.has("ALERTA", motivo="sensor_inacessivel", sensor="attention")
    # recupera a acessibilidade e a regra volta a avaliar
    sim.sensors["attention"].accessible = True
    sim.set_sensor("attention", 15.0)
    engine.tick()
    assert ledger.has("transicao", forma="Sentinela", para="equilibrium")


def test_form_without_source_path_generates_no_reading_nor_failure(engine, ledger):
    """Exemplos 5/6 da FORMAL: formas podem não declarar source_path."""
    _load(engine,
          ir.form("Piscada", "event", "impulso_curto", "2s"))
    engine.tick()
    assert "Piscada" in engine.forms  # sem crash, sem leitura
    assert not ledger.has("ALERTA", motivo="sensor_nao_registrado")
