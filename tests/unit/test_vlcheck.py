# -*- coding: utf-8 -*-
"""Testes do mini-validador de superfície (tests/vlcheck.py).

Ancoragem: os exemplos 1–6 da FORMAL §5 devem validar SEM erros (são os
programas canônicos da linguagem); as cláusulas de erro textuais devem
produzir o diagnóstico correspondente. O vlcheck é o verificador sintático
do banco de 20 prompts (PLAN §7).
"""

from __future__ import annotations

import vlcheck

# Exemplos canônicos — docs/FORMAL.md §5
EXAMPLE_1 = '''
nonequilibrium PensarLivre {
    value: "consciencia_anteneoliberal_ativa",
    horizon: 60s,
    source_path: "attention",
    maintenance_deadline: 3s,
    exchange_mode: "cooperation"
}

review PensarLivre {
    when attention < 30% -> reclassify_as_equilibrium
}
'''

EXAMPLE_2 = '''
nonequilibrium TradingEspeculativo {
    value: "lucro_arbitragem_alta_frequencia",
    horizon: 7s,
    source_path: "cpu_temp",
    maintenance_deadline: 2s,
    exchange_mode: "extraction"
}

review TradingEspeculativo {
    when cpu_temp > 85°C -> subvert,
                            act(CpuPowerCap, 50)
}
'''

EXAMPLE_3 = '''
nonequilibrium ServidorCritico {
    value: "processamento_contínuo",
    horizon: 3600s,
    source_path: "cpu_temp",
    maintenance_deadline: 10s,
    exchange_mode: "cooperation"
}

review ServidorCritico {
    when cpu_temp > 70°C -> act(Ventoinha, 200)
}
'''

EXAMPLE_4 = '''
nonequilibrium TarefaImportante {
    value: "dados_sensiveis",
    horizon: 30s,
    source_path: "cpu_power",
    maintenance_deadline: 5s,
    exchange_mode: "cooperation"
}

main {
    every 4s { keep(TarefaImportante) },
    every 10s { act(LedIndicador, "verde") }
}
'''

EXAMPLE_5 = '''
event Piscada {
    value: "impulso_curto",
    horizon: 2s
}

review Piscada {
    when cpu_temp > 90°C -> dissolve
}
'''

EXAMPLE_6 = '''
equilibrium Registro {
    value: "documento_persistente",
    horizon: 86400s,
    cost_bytes: 4096
}
'''


def _codes(text):
    return {e.code for e in vlcheck.validate(text)}


def test_canonical_formal_examples_validate_without_errors():
    for i, example in enumerate([EXAMPLE_1, EXAMPLE_2, EXAMPLE_3,
                                 EXAMPLE_4, EXAMPLE_5, EXAMPLE_6], 1):
        errors = vlcheck.validate(example)
        assert errors == [], f"Exemplo {i} da FORMAL §5 rejeitado: {errors}"


def test_block_and_line_comments_are_ignored():
    text = '''
    /* comentário
       de bloco */
    event X { value: "v", horizon: 3s } // comentário de linha
    '''
    assert vlcheck.validate(text) == []


def test_trailing_comma_is_rejected():
    text = 'event X { value: "v", horizon: 3s, }'
    assert "virgula_final" in _codes(text)


def test_duration_without_unit_is_rejected():
    text = 'event X { value: "v", horizon: 3 }'
    assert "duracao_invalida" in _codes(text)


def test_string_above_256_bytes_is_rejected():
    text = f'event X {{ value: "{"a" * 300}", horizon: 3s }}'
    assert "string_muito_longa" in _codes(text)


def test_unknown_action_is_rejected():
    text = '''
    event X { value: "v", horizon: 3s }
    review X { when cpu_temp > 90°C -> explodir }
    '''
    assert "acao_desconhecida" in _codes(text)


def test_invalid_operator_is_rejected():
    text = '''
    event X { value: "v", horizon: 3s }
    review X { when cpu_temp !== 90°C -> dissolve }
    '''
    assert {"operador_invalido", "lexema_invalido"} & _codes(text)


def test_unknown_attribute_is_rejected():
    text = 'event X { value: "v", horizon: 3s, magia: "negra" }'
    assert "atributo_desconhecido" in _codes(text)


def test_program_persisted_by_runtime_validates(engine, sim, tmp_path):
    """O `.vl` gravado pela persistência (FORMAL §4.1) é reparseável."""
    from fxp_sim import ir, loader
    loader.load(engine, ir.program(
        ir.form("Doc", "nonequilibrium", "estado_importante", "60s",
                source_path="attention", maintenance_deadline="3s"),
        ir.review("Doc", ir.rule("attention", "<", 30, "%",
                                 ir.action("reclassify_as_equilibrium"))),
    ))
    sim.set_sensor("attention", 10.0)
    engine.tick()  # persiste como equilibrium
    path = tmp_path / "persistence" / "Doc.vl"
    errors = vlcheck.validate(path.read_text(encoding="utf-8"))
    assert errors == []
