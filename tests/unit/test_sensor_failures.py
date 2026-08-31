# -*- coding: utf-8 -*-
"""Testes de Falha Controlada de sensores (PLAN §1.2) — FORMAL §4.7.

(i) leitura `0.0` é válida e avaliada normalmente;
(ii) sensor ausente ou inacessível: nenhuma `when` é avaliada, o Caderno
     registra alerta e não há disparo falso — sensor ausente NUNCA é 0.0.
"""

from __future__ import annotations

from fxp_sim import ir, loader


def _carregar(engine, *declaracoes):
    return loader.carregar(engine, ir.programa(*declaracoes))


def _forma_atencao():
    return (
        ir.forma("Sentinela", "nonequilibrium", "vigia", "30s",
                 source_path="attention", maintenance_deadline="3s"),
        ir.review("Sentinela",
                  ir.regra("attention", "<", 30, "%",
                           ir.acao("reclassify_as_equilibrium"))),
    )


def test_leitura_zero_e_valida_e_dispara_regras(engine, cad, sim):
    """0.0 é leitura física legítima: a regra `attention < 30%` DEVE disparar."""
    _carregar(engine, *_forma_atencao())
    sim.set_sensor("attention", 0.0)
    engine.tick()
    assert cad.tem("transicao", forma="Sentinela", para="equilibrium")
    leituras = [e for e in cad.buscar("LEITURA", sensor="attention")]
    assert leituras and leituras[-1]["valor"] == 0.0
    # zero NÃO é falha de I/O: nenhum alerta de sensor
    assert not cad.tem("ALERTA", motivo="sensor_nao_registrado")
    assert not cad.tem("ALERTA", motivo="sensor_inacessivel")


def test_leitura_zero_nunca_e_falha_de_io(engine, cad, sim):
    sim.set_sensor("cpu_temp", 0.0)
    assert sim.read_sensor("cpu_temp") == 0.0  # valor, não None
    assert not cad.tem("ALERTA")


def test_sensor_ausente_nao_avalia_condicao_nem_dispara(engine, cad, sim):
    """FORMAL §4.7: sensor fora do registro é falha de I/O — a condição não
    é avaliada naquele tick e não há disparo falso (o valor NÃO é 0.0)."""
    _carregar(
        engine,
        ir.forma("Fantasma", "nonequilibrium", "obs", "30s",
                 source_path="sensor_inexistente", maintenance_deadline="3s"),
        ir.review("Fantasma",
                  ir.regra("sensor_inexistente", "<", 30, "%",
                           ir.acao("reclassify_as_equilibrium"))),
    )
    for _ in range(4):
        engine.tick()
    # a regra jamais avaliou: forma permanece nonequilibrium
    assert engine.forms["Fantasma"].conjugation == "nonequilibrium"
    assert not cad.tem("transicao")
    # alerta registrado a cada tick de leitura
    alertas = cad.buscar("ALERTA", motivo="sensor_nao_registrado",
                         sensor="sensor_inexistente")
    assert len(alertas) >= 1


def test_sensor_ausente_nao_e_tratado_como_zero(engine, cad, sim):
    """Se sensor ausente valesse 0.0, a regra `attention < 30%` dispararia
    falsamente — é exatamente o que a FORMAL §4.7 proíbe."""
    _carregar(engine, *_forma_atencao())
    sim.desregistrar_sensor("attention")
    for _ in range(3):
        engine.tick()
    assert not cad.tem("transicao")  # nenhum disparo falso
    assert engine.forms["Sentinela"].conjugation == "nonequilibrium"


def test_sensor_registrado_inacessivel_segue_a_mesma_regra(engine, cad, sim):
    """FORMAL §4.7: registrado porém inacessível (falha de leitura em modo
    real) segue a mesma regra — condição não avaliada + alerta."""
    _carregar(engine, *_forma_atencao())
    sim.falhar_sensor("attention")
    engine.tick()
    assert not cad.tem("transicao")
    assert cad.tem("ALERTA", motivo="sensor_inacessivel", sensor="attention")
    # recupera a acessibilidade e a regra volta a avaliar
    sim.sensores["attention"].acessivel = True
    sim.set_sensor("attention", 15.0)
    engine.tick()
    assert cad.tem("transicao", forma="Sentinela", para="equilibrium")


def test_forma_sem_source_path_nao_gera_leitura_nem_falha(engine, cad):
    """Exemplos 5/6 da FORMAL: formas podem não declarar source_path."""
    _carregar(engine,
              ir.forma("Piscada", "event", "impulso_curto", "2s"))
    engine.tick()
    assert "Piscada" in engine.forms  # sem crash, sem leitura
    assert not cad.tem("ALERTA", motivo="sensor_nao_registrado")
