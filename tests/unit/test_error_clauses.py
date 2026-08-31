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

SENSOR_REGISTRY = {
    "cpu_temp": {"quantity": "temperature", "unit": "°C"},
    "cpu_power": {"quantity": "power", "unit": "W"},
    "attention": {"quantity": "attention", "unit": "%"},
}
ACTOR_REGISTRY = {"CpuPowerCap": {}, "Fan": {}, "StatusLed": {}}


def _codes(diagnoses):
    return {d.code for d in diagnoses}


# ----------------------------------------------------------------------
# Forma sem `value` / `horizon` (FORMAL §3; Lei 1 do MANIFESTO)
# ----------------------------------------------------------------------
def test_form_without_value_rejected_in_text():
    text = 'event X { horizon: 3s }'
    errors = vlcheck.validate(text)
    assert "value_obrigatorio" in {e.code for e in errors}


def test_form_without_horizon_rejected_in_text():
    text = 'event X { value: "v" }'
    errors = vlcheck.validate(text)
    assert "horizon_obrigatorio" in {e.code for e in errors}


def test_value_before_horizon_required_in_text():
    text = 'event X { horizon: 3s, value: "v" }'
    errors = vlcheck.validate(text)
    assert "ordem_value_horizon" in {e.code for e in errors}


def test_form_without_value_or_horizon_rejected_in_ir():
    form = {"type": "form", "conjugation": "event", "name": "X",
            "attributes": {"horizon": "3s"}}
    assert "atributo_obrigatorio_ausente" in _codes(
        contract.validate_program(ir.program(form)))


# ----------------------------------------------------------------------
# Atributos por conjugação (FORMAL §3)
# ----------------------------------------------------------------------
def test_nonequilibrium_without_maintenance_deadline_rejected():
    form = ir.form("X", "nonequilibrium", "v", "3s")  # sem deadline
    codes = _codes(contract.validate_program(ir.program(form)))
    assert {"maintenance_deadline_ausente"} & codes


def test_cost_bytes_outside_equilibrium_rejected():
    form = ir.form("X", "event", "v", "3s", cost_bytes=16)
    codes = _codes(contract.validate_program(ir.program(form)))
    assert "atributo_nao_aplicavel" in codes
    errors = vlcheck.validate('event X { value: "v", horizon: 3s, cost_bytes: 16 }')
    assert "atributo_nao_aplicavel" in {e.code for e in errors}


def test_exchange_mode_outside_nonequilibrium_rejected():
    form = ir.form("X", "equilibrium", "v", "3s", exchange_mode="cooperation")
    codes = _codes(contract.validate_program(ir.program(form)))
    assert "atributo_nao_aplicavel" in codes


# ----------------------------------------------------------------------
# Review órfã / duplicada (FORMAL §3: erros de compilação)
# ----------------------------------------------------------------------
def test_orphan_review_rejected_in_text():
    text = '''
    event X { value: "v", horizon: 3s }
    review Y { when cpu_temp > 90°C -> dissolve }
    '''
    errors = vlcheck.validate(text)
    assert "review_orfa" in {e.code for e in errors}


def test_duplicate_review_rejected_in_text():
    text = '''
    event X { value: "v", horizon: 3s }
    review X { when cpu_temp > 90°C -> dissolve }
    review X { when cpu_temp < 10°C -> dissolve }
    '''
    errors = vlcheck.validate(text)
    assert "review_duplicada" in {e.code for e in errors}


def test_orphan_review_rejected_in_ir():
    review = ir.review("Fantasma", ir.rule("cpu_temp", ">", 90, "°C",
                                           ir.action("dissolve")))
    form = ir.form("X", "event", "v", "3s")
    codes = _codes(contract.validate_program(ir.program(form, review)))
    assert "review_orfa" in codes


def test_duplicate_review_rejected_in_ir():
    form = ir.form("X", "event", "v", "3s")
    r1 = ir.review("X", ir.rule("cpu_temp", ">", 90, "°C", ir.action("dissolve")))
    r2 = ir.review("X", ir.rule("cpu_temp", "<", 10, "°C", ir.action("dissolve")))
    codes = _codes(contract.validate_program(ir.program(form, r1, r2)))
    assert "review_duplicada" in codes


# ----------------------------------------------------------------------
# `keep` de forma inexistente (cláusula de erro — AGENTS.md)
# ----------------------------------------------------------------------
def test_keep_of_nonexistent_form_rejected_in_text():
    text = '''
    event X { value: "v", horizon: 3s }
    main { every 1s { keep(Inexistente) } }
    '''
    errors = vlcheck.validate(text)
    assert "keep_forma_inexistente" in {e.code for e in errors}


def test_keep_of_nonexistent_form_rejected_in_ir():
    form = ir.form("X", "nonequilibrium", "v", "30s",
                   source_path="cpu_power", maintenance_deadline="2s")
    main = ir.main_block(ir.every("1s", ir.keep_("Outra")))
    codes = _codes(contract.validate_program(ir.program(form, main=main)))
    assert "keep_forma_inexistente" in codes


def test_keep_of_dissolved_form_recorded_at_runtime(engine, ledger):
    """Em runtime, keep para forma já dissolvida é registrado e não quebra."""
    interpreter = _load_with_main(engine)
    engine.tick()  # t=1: vence o primeiro `every 1s`
    engine.dissolve_form("Solo", fim="collapse_maintenance")
    interpreter.run_due()
    assert ledger.has("keep_unknown_form", forma="Solo")


def _load_with_main(engine):
    from fxp_sim import loader
    program = ir.program(
        ir.form("Solo", "nonequilibrium", "v", "30s",
                source_path="cpu_power", maintenance_deadline="2s"),
        main=ir.main_block(ir.every("1s", ir.keep_("Solo"))),
    )
    return loader.load(engine, program)


# ----------------------------------------------------------------------
# Sensor ausente (§4.7 — coberto em runtime; unidade/registro no IR)
# ----------------------------------------------------------------------
def test_unregistered_sensor_detected_in_ir():
    form = ir.form("X", "nonequilibrium", "v", "3s",
                   source_path="solar_flare", maintenance_deadline="2s")
    codes = _codes(contract.validate_program(
        ir.program(form), sensors=SENSOR_REGISTRY))
    assert "sensor_nao_registrado" in codes


def test_source_path_with_os_path_rejected():
    form = ir.form("X", "nonequilibrium", "v", "3s",
                   source_path="/sys/class/thermal/thermal_zone0/temp",
                   maintenance_deadline="2s")
    codes = _codes(contract.validate_program(ir.program(form)))
    assert "source_path_nao_simbolico" in codes
    errors = vlcheck.validate(
        'nonequilibrium X { value: "v", horizon: 3s, '
        'source_path: "/sys/class/thermal/temp", maintenance_deadline: 2s }')
    assert "source_path_nao_simbolico" in {e.code for e in errors}


def test_unit_incompatible_with_quantity_rejected():
    form = ir.form("X", "event", "v", "3s")
    review = ir.review("X", ir.rule("cpu_temp", ">", 90, "W",
                                    ir.action("dissolve")))
    codes = _codes(contract.validate_program(
        ir.program(form, review), sensors=SENSOR_REGISTRY))
    assert "unidade_incompativel" in codes


# ----------------------------------------------------------------------
# Ator inexistente / valor fora de limite (contract e runtime)
# ----------------------------------------------------------------------
def test_unregistered_actor_detected_in_ir():
    form = ir.form("X", "event", "v", "3s")
    review = ir.review("X", ir.rule("cpu_temp", ">", 90, "°C",
                                    ir.act_("AtorFantasma", 5)))
    codes = _codes(contract.validate_program(
        ir.program(form, review), sensors=SENSOR_REGISTRY,
        actors=ACTOR_REGISTRY))
    assert "ator_nao_registrado" in codes


def test_reclassify_to_nonequilibrium_without_declared_deadline(engine, ledger):
    """FORMAL §3: é erro de RUNTIME registrado no Caderno; a forma permanece
    como estava (equilibrium nunca declarou deadline)."""
    program = ir.program(
        ir.form("Doc", "equilibrium", "v", "60s"),
        ir.review("Doc", ir.rule("cpu_temp", ">", 90, "°C",
                                 ir.action("reclassify_as_nonequilibrium"))),
    )
    from fxp_sim import loader
    loader.load(engine, program)
    engine.fxp.set_sensor("cpu_temp", 95.0)
    engine.tick()
    assert ledger.has("reclassify_no_deadline", forma="Doc")
    assert engine.forms["Doc"].conjugation == "equilibrium"  # permaneceu


def test_reclassify_to_nonequilibrium_with_declared_deadline(engine, ledger):
    """NEQ -> EQ -> NEQ é legal: o deadline declarado sobrevive."""
    program = ir.program(
        ir.form("P", "nonequilibrium", "v", "60s", source_path="attention",
                maintenance_deadline="3s", exchange_mode="extraction"),
        ir.review("P", ir.rule("attention", "<", 30, "%",
                               ir.action("reclassify_as_equilibrium")),
                      ir.rule("attention", ">", 80, "%",
                              ir.action("reclassify_as_nonequilibrium"))),
    )
    from fxp_sim import loader
    loader.load(engine, program)
    engine.fxp.set_sensor("attention", 15.0)
    engine.tick()  # NEQ -> EQ
    engine.fxp.set_sensor("attention", 90.0)
    engine.tick()  # EQ -> NEQ (deadline 3s declarado preservado)
    assert ledger.has("transition", forma="P", para="nonequilibrium")
    assert engine.forms["P"].conjugation == "nonequilibrium"
    assert engine.forms["P"].maintenance_deadline == 3.0
