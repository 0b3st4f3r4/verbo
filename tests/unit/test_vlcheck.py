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
EXEMPLO_1 = '''
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

EXEMPLO_2 = '''
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

EXEMPLO_3 = '''
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

EXEMPLO_4 = '''
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

EXEMPLO_5 = '''
event Piscada {
    value: "impulso_curto",
    horizon: 2s
}

review Piscada {
    when cpu_temp > 90°C -> dissolve
}
'''

EXEMPLO_6 = '''
equilibrium Registro {
    value: "documento_persistente",
    horizon: 86400s,
    cost_bytes: 4096
}
'''


def _codigos(texto):
    return {e.codigo for e in vlcheck.validar(texto)}


def test_exemplos_canonicos_da_formal_validam_sem_erros():
    for i, exemplo in enumerate([EXEMPLO_1, EXEMPLO_2, EXEMPLO_3,
                                 EXEMPLO_4, EXEMPLO_5, EXEMPLO_6], 1):
        erros = vlcheck.validar(exemplo)
        assert erros == [], f"Exemplo {i} da FORMAL §5 rejeitado: {erros}"


def test_comentarios_de_bloco_e_linha_sao_ignorados():
    texto = '''
    /* comentário
       de bloco */
    event X { value: "v", horizon: 3s } // comentário de linha
    '''
    assert vlcheck.validar(texto) == []


def test_virgula_final_e_rejeitada():
    texto = 'event X { value: "v", horizon: 3s, }'
    assert "virgula_final" in _codigos(texto)


def test_duracao_sem_unidade_e_rejeitada():
    texto = 'event X { value: "v", horizon: 3 }'
    assert "duracao_invalida" in _codigos(texto)


def test_string_acima_de_256_bytes_e_rejeitada():
    texto = f'event X {{ value: "{"a" * 300}", horizon: 3s }}'
    assert "string_muito_longa" in _codigos(texto)


def test_acao_desconhecida_e_rejeitada():
    texto = '''
    event X { value: "v", horizon: 3s }
    review X { when cpu_temp > 90°C -> explodir }
    '''
    assert "acao_desconhecida" in _codigos(texto)


def test_operador_invalido_e_rejeitado():
    texto = '''
    event X { value: "v", horizon: 3s }
    review X { when cpu_temp !== 90°C -> dissolve }
    '''
    assert {"operador_invalido", "lexema_invalido"} & _codigos(texto)


def test_atributo_desconhecido_e_rejeitado():
    texto = 'event X { value: "v", horizon: 3s, magia: "negra" }'
    assert "atributo_desconhecido" in _codigos(texto)


def test_programa_persistido_pelo_runtime_valida(engine, sim, tmp_path):
    """O `.vl` gravado pela persistência (FORMAL §4.1) é reparseável."""
    from fxp_sim import ir, loader
    loader.carregar(engine, ir.programa(
        ir.forma("Doc", "nonequilibrium", "estado_importante", "60s",
                 source_path="attention", maintenance_deadline="3s"),
        ir.review("Doc", ir.regra("attention", "<", 30, "%",
                                  ir.acao("reclassify_as_equilibrium"))),
    ))
    sim.set_sensor("attention", 10.0)
    engine.tick()  # persiste como equilibrium
    caminho = tmp_path / "persistencia" / "Doc.vl"
    erros = vlcheck.validar(caminho.read_text(encoding="utf-8"))
    assert erros == []
