# -*- coding: utf-8 -*-
"""≥ 1 teste por cláusula de erro da FORMAL — critério de "Done" da Etapa 1
(AGENTS.md §2.2): sensor ausente, ator inexistente, valor fora de limite,
forma sem `value`/`horizon`, review órfã/duplicada, `keep` de forma
inexistente.

Duas camadas são testadas:
- TEXTO (tests/vlcheck.py): o que o parser da Etapa 2 rejeitará na compilação
  (FORMAL §3: "erros de compilação");
- IR/registro (tests/fxp_sim/contract.py) e RUNTIME (protótipo): o contrato
  estrutural e os erros de runtime registrados no Caderno.
"""

from __future__ import annotations

from fxp_sim import contract, ir
import vlcheck  # tests/vlcheck.py (validador de superfície .vl)

REGISTRO_SENSORES = {
    "cpu_temp": {"grandeza": "temperatura", "unidade": "°C"},
    "cpu_power": {"grandeza": "potencia", "unidade": "W"},
    "attention": {"grandeza": "atencao", "unidade": "%"},
}
REGISTRO_ATORES = {"CpuPowerCap": {}, "Ventoinha": {}, "LedIndicador": {}}


def _codigos(diagnosticos):
    return {d.codigo for d in diagnosticos}


# ----------------------------------------------------------------------
# Forma sem `value` / `horizon` (FORMAL §3; Lei 1 do MANIFESTO)
# ----------------------------------------------------------------------
def test_forma_sem_value_rejeitada_no_texto():
    texto = 'event X { horizon: 3s }'
    erros = vlcheck.validar(texto)
    assert "value_obrigatorio" in {e.codigo for e in erros}


def test_forma_sem_horizon_rejeitada_no_texto():
    texto = 'event X { value: "v" }'
    erros = vlcheck.validar(texto)
    assert "horizon_obrigatorio" in {e.codigo for e in erros}


def test_value_antes_de_horizon_e_exigido_no_texto():
    texto = 'event X { horizon: 3s, value: "v" }'
    erros = vlcheck.validar(texto)
    assert "ordem_value_horizon" in {e.codigo for e in erros}


def test_forma_sem_value_ou_horizon_rejeitada_no_ir():
    forma = {"tipo": "forma", "conjugacao": "event", "nome": "X",
             "atributos": {"horizon": "3s"}}
    assert "atributo_obrigatorio_ausente" in _codigos(
        contract.validar_programa(ir.programa(forma)))


# ----------------------------------------------------------------------
# Atributos por conjugação (FORMAL §3)
# ----------------------------------------------------------------------
def test_nonequilibrium_sem_maintenance_deadline_rejeitado():
    forma = ir.forma("X", "nonequilibrium", "v", "3s")  # sem deadline
    codigos = _codigos(contract.validar_programa(ir.programa(forma)))
    assert {"maintenance_deadline_ausente"} & codigos


def test_cost_bytes_fora_de_equilibrium_rejeitado():
    forma = ir.forma("X", "event", "v", "3s", cost_bytes=16)
    codigos = _codigos(contract.validar_programa(ir.programa(forma)))
    assert "atributo_nao_aplicavel" in codigos
    erros = vlcheck.validar('event X { value: "v", horizon: 3s, cost_bytes: 16 }')
    assert "atributo_nao_aplicavel" in {e.codigo for e in erros}


def test_exchange_mode_fora_de_nonequilibrium_rejeitado():
    forma = ir.forma("X", "equilibrium", "v", "3s", exchange_mode="cooperation")
    codigos = _codigos(contract.validar_programa(ir.programa(forma)))
    assert "atributo_nao_aplicavel" in codigos


# ----------------------------------------------------------------------
# Review órfã / duplicada (FORMAL §3: erros de compilação)
# ----------------------------------------------------------------------
def test_review_orfa_rejeitada_no_texto():
    texto = '''
    event X { value: "v", horizon: 3s }
    review Y { when cpu_temp > 90°C -> dissolve }
    '''
    erros = vlcheck.validar(texto)
    assert "review_orfa" in {e.codigo for e in erros}


def test_review_duplicada_rejeitada_no_texto():
    texto = '''
    event X { value: "v", horizon: 3s }
    review X { when cpu_temp > 90°C -> dissolve }
    review X { when cpu_temp < 10°C -> dissolve }
    '''
    erros = vlcheck.validar(texto)
    assert "review_duplicada" in {e.codigo for e in erros}


def test_review_orfa_rejeitada_no_ir():
    review = ir.review("Fantasma", ir.regra("cpu_temp", ">", 90, "°C",
                                            ir.acao("dissolve")))
    forma = ir.forma("X", "event", "v", "3s")
    codigos = _codigos(contract.validar_programa(ir.programa(forma, review)))
    assert "review_orfa" in codigos


def test_review_duplicada_rejeitada_no_ir():
    forma = ir.forma("X", "event", "v", "3s")
    r1 = ir.review("X", ir.regra("cpu_temp", ">", 90, "°C", ir.acao("dissolve")))
    r2 = ir.review("X", ir.regra("cpu_temp", "<", 10, "°C", ir.acao("dissolve")))
    codigos = _codigos(contract.validar_programa(ir.programa(forma, r1, r2)))
    assert "review_duplicada" in codigos


# ----------------------------------------------------------------------
# `keep` de forma inexistente (cláusula de erro — AGENTS.md)
# ----------------------------------------------------------------------
def test_keep_de_forma_inexistente_rejeitado_no_texto():
    texto = '''
    event X { value: "v", horizon: 3s }
    main { every 1s { keep(Inexistente) } }
    '''
    erros = vlcheck.validar(texto)
    assert "keep_forma_inexistente" in {e.codigo for e in erros}


def test_keep_de_forma_inexistente_rejeitado_no_ir():
    forma = ir.forma("X", "nonequilibrium", "v", "30s",
                     source_path="cpu_power", maintenance_deadline="2s")
    main = ir.main_bloco(ir.every("1s", ir.keep_("Outra")))
    codigos = _codigos(contract.validar_programa(ir.programa(forma, main=main)))
    assert "keep_forma_inexistente" in codigos


def test_keep_de_forma_dissolvida_registrado_em_runtime(engine, cad):
    """Em runtime, keep para forma já dissolvida é registrado e não quebra."""
    interpretador = _carregar_com_main(engine)
    engine.tick()  # t=1: vence o primeiro `every 1s`
    engine.dissolve_form("Solo", fim="collapse_maintenance")
    interpretador.run_due()
    assert cad.tem("keep_forma_inexistente", forma="Solo")


def _carregar_com_main(engine):
    from fxp_sim import loader
    programa = ir.programa(
        ir.forma("Solo", "nonequilibrium", "v", "30s",
                 source_path="cpu_power", maintenance_deadline="2s"),
        main=ir.main_bloco(ir.every("1s", ir.keep_("Solo"))),
    )
    return loader.carregar(engine, programa)


# ----------------------------------------------------------------------
# Sensor ausente (§4.7 — coberto em runtime; unidade/registro no IR)
# ----------------------------------------------------------------------
def test_sensor_nao_registrado_detectado_no_ir():
    forma = ir.forma("X", "nonequilibrium", "v", "3s",
                     source_path="solar_flare", maintenance_deadline="2s")
    codigos = _codigos(contract.validar_programa(
        ir.programa(forma), sensores=REGISTRO_SENSORES))
    assert "sensor_nao_registrado" in codigos


def test_source_path_com_caminho_de_so_rejeitado():
    forma = ir.forma("X", "nonequilibrium", "v", "3s",
                     source_path="/sys/class/thermal/thermal_zone0/temp",
                     maintenance_deadline="2s")
    codigos = _codigos(contract.validar_programa(ir.programa(forma)))
    assert "source_path_nao_simbolico" in codigos
    erros = vlcheck.validar(
        'nonequilibrium X { value: "v", horizon: 3s, '
        'source_path: "/sys/class/thermal/temp", maintenance_deadline: 2s }')
    assert "source_path_nao_simbolico" in {e.codigo for e in erros}


def test_unidade_incompativel_com_grandeza_rejeitada():
    forma = ir.forma("X", "event", "v", "3s")
    review = ir.review("X", ir.regra("cpu_temp", ">", 90, "W",
                                     ir.acao("dissolve")))
    codigos = _codigos(contract.validar_programa(
        ir.programa(forma, review), sensores=REGISTRO_SENSORES))
    assert "unidade_incompativel" in codigos


# ----------------------------------------------------------------------
# Ator inexistente / valor fora de limite (contract e runtime)
# ----------------------------------------------------------------------
def test_ator_nao_registrado_detectado_no_ir():
    forma = ir.forma("X", "event", "v", "3s")
    review = ir.review("X", ir.regra("cpu_temp", ">", 90, "°C",
                                     ir.act_("AtorFantasma", 5)))
    codigos = _codigos(contract.validar_programa(
        ir.programa(forma, review), sensores=REGISTRO_SENSORES,
        atores=REGISTRO_ATORES))
    assert "ator_nao_registrado" in codigos


def test_reclassify_para_nonequilibrium_sem_deadline_declarado(engine, cad):
    """FORMAL §3: é erro de RUNTIME registrado no Caderno; a forma permanece
    como estava (equilibrium nunca declarou deadline)."""
    programa = ir.programa(
        ir.forma("Doc", "equilibrium", "v", "60s"),
        ir.review("Doc", ir.regra("cpu_temp", ">", 90, "°C",
                                  ir.acao("reclassify_as_nonequilibrium"))),
    )
    from fxp_sim import loader
    loader.carregar(engine, programa)
    engine.fxp.set_sensor("cpu_temp", 95.0)
    engine.tick()
    assert cad.tem("reclassify_sem_deadline", forma="Doc")
    assert engine.forms["Doc"].conjugation == "equilibrium"  # permaneceu


def test_reclassify_para_nonequilibrium_com_deadline_declarado(engine, cad):
    """NEQ -> EQ -> NEQ é legal: o deadline declarado sobrevive."""
    programa = ir.programa(
        ir.forma("P", "nonequilibrium", "v", "60s", source_path="attention",
                 maintenance_deadline="3s", exchange_mode="extraction"),
        ir.review("P", ir.regra("attention", "<", 30, "%",
                                ir.acao("reclassify_as_equilibrium")),
                      ir.regra("attention", ">", 80, "%",
                               ir.acao("reclassify_as_nonequilibrium"))),
    )
    from fxp_sim import loader
    loader.carregar(engine, programa)
    engine.fxp.set_sensor("attention", 15.0)
    engine.tick()  # NEQ -> EQ
    engine.fxp.set_sensor("attention", 90.0)
    engine.tick()  # EQ -> NEQ (deadline 3s declarado preservado)
    assert cad.tem("transicao", forma="P", para="nonequilibrium")
    assert engine.forms["P"].conjugation == "nonequilibrium"
    assert engine.forms["P"].maintenance_deadline == 3.0
