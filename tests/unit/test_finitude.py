# -*- coding: utf-8 -*-
"""Asserções de Finitude (PLAN §1.2) — horizontes, prazos e manutenção.

Rastreabilidade: FORMAL §4.1 (conjugações, horizon absoluto, manutenção),
§4.2 (ordem no tick), AGENTS.md "Done" da Etapa 1.
"""

from __future__ import annotations

from fxp_sim import ir, loader


def _carregar(engine, *declaracoes, main=None):
    return loader.carregar(engine, ir.programa(*declaracoes, main=main))


# ----------------------------------------------------------------------
# Horizonte de `event` (Asserções de Finitude — PLAN §1.2)
# ----------------------------------------------------------------------
def test_event_expira_no_horizon_com_fim_tipificado(engine, cad):
    _carregar(engine, ir.forma("Piscada", "event", "impulso_curto", "3s"))
    engine.tick()  # t=1
    engine.tick()  # t=2 — ainda ativa
    assert "Piscada" in engine.forms
    engine.tick()  # t=3 — horizon esgota (>=)
    assert "Piscada" not in engine.forms
    assert cad.tem("dissolve_horizon", forma="Piscada")


def test_horizon_e_absoluto_reclassificacao_nao_renova(engine, cad, sim):
    """FORMAL §4.1: horizon é contado desde a criação; reclassificar não o
    renova. NEQ (horizon 5s) vira EQ em t=1 e dissolve em t=5."""
    _carregar(
        engine,
        ir.forma("Pensar", "nonequilibrium", "ideia", "5s",
                 source_path="attention", maintenance_deadline="3s"),
        ir.review("Pensar",
                  ir.regra("attention", "<", 30, "%",
                           ir.acao("reclassify_as_equilibrium"))),
    )
    sim.set_sensor("attention", 15.0)
    engine.tick()  # t=1: reclassifica para equilibrium (persistida)
    assert cad.tem("transicao", forma="Pensar", para="equilibrium")
    assert engine.forms["Pensar"].creation_time == 0.0  # não renovado
    sim.set_sensor("attention", 90.0)  # regra para de disparar
    for _ in range(3):  # t=2,3,4
        engine.tick()
    assert "Pensar" in engine.forms  # t=4: ainda viva
    engine.tick()  # t=5: horizon original esgota
    assert "Pensar" not in engine.forms
    assert cad.tem("dissolve_horizon", forma="Pensar")


def test_equilibrium_tambem_expira_por_horizon(engine, cad):
    """Matriz de transições (FORMAL §4.1): EQ -> DIS por horizon."""
    _carregar(engine, ir.forma("Registro", "equilibrium", "doc", "2s"))
    engine.tick()
    assert "Registro" in engine.forms  # t=1: 1 >= 2? não
    engine.tick()  # t=2: 2 >= 2 — horizon esgota no próprio limite
    assert "Registro" not in engine.forms
    assert cad.tem("dissolve_horizon", forma="Registro")


# ----------------------------------------------------------------------
# Manutenção de `nonequilibrium` (FORMAL §4.1)
# ----------------------------------------------------------------------
def test_nonequilibrium_com_regra_ativa_tem_manutencao_implicita(engine, cad):
    """FORMAL §4.1(ii): manutenção implícita a cada tick enquanto houver
    regra de revisão ativa — a forma sobrevive além do deadline sem keep()."""
    _carregar(
        engine,
        ir.forma("Vigilia", "nonequilibrium", "trabalho", "30s",
                 source_path="attention", maintenance_deadline="2s"),
        ir.review("Vigilia", ir.regra("attention", "<", 5, "%", ir.acao("dissolve"))),
    )
    for _ in range(6):  # attention = 100 -> regra nunca dispara
        engine.tick()
    assert "Vigilia" in engine.forms
    assert not cad.tem("collapse_maintenance")


def test_nonequilibrium_sem_manutencao_colapsa_no_primeiro_vencimento(engine, cad):
    """Sem regra ativa e sem keep(): colapsa no primeiro vencimento do
    maintenance_deadline ('exceder' = estritamente maior, FORMAL §4.1)."""
    _carregar(
        engine,
        ir.forma("Solo", "nonequilibrium", "sem_vigilia", "30s",
                 source_path="cpu_power", maintenance_deadline="2s"),
    )
    engine.tick()  # t=1: 1 > 2? não
    engine.tick()  # t=2: 2 > 2? não (limite estrito)
    assert "Solo" in engine.forms
    engine.tick()  # t=3: 3 > 2? sim — colapso
    assert "Solo" not in engine.forms
    assert cad.tem("collapse_maintenance", forma="Solo")


def test_keep_manual_renova_o_prazo(engine, cad):
    """FORMAL §4.1(i): keep(forma) explícito renova o prazo de manutenção."""
    interpretador = _carregar(
        engine,
        ir.forma("Solo", "nonequilibrium", "mantido", "30s",
                 source_path="cpu_power", maintenance_deadline="2s"),
        main=ir.main_bloco(ir.every("1s", ir.keep_("Solo"))),
    )
    for _ in range(6):
        interpretador.run_due()
        engine.tick()
    assert "Solo" in engine.forms
    assert not cad.tem("collapse_maintenance")


# ----------------------------------------------------------------------
# Fim único e ordem no tick (FORMAL §4.2)
# ----------------------------------------------------------------------
def test_forma_termina_uma_unicamente_por_tick(engine, cad, sim):
    """Regra que dissolve no mesmo tick do vencimento do horizon: a forma
    termina UMA vez, com o fim da regra (avaliada antes dos prazos)."""
    _carregar(
        engine,
        ir.forma("Ciclo", "nonequilibrium", "lucro", "3s",
                 source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Ciclo", ir.regra("cpu_temp", ">", 85, "°C",
                                    ir.acao("dissolve"))),
    )
    sim.set_sensor("cpu_temp", 90.0)
    engine.tick()  # t=3 não chegou; regra dispara primeiro
    assert "Ciclo" not in engine.forms
    assert cad.kinds().count("dissolve_rule") == 1
    assert not cad.tem("dissolve_horizon")


# ----------------------------------------------------------------------
# Contadores de retenção do runtime (proxy de heap — PLAN §1.2 iii)
# ----------------------------------------------------------------------
def test_contadores_de_retencao_dentro_dos_orcamentos(engine, cad, vbl):
    _carregar(
        engine,
        ir.forma("Ev", "event", "curto", "3s"),
        ir.forma("Eq", "equilibrium", "doc", "30s", cost_bytes=128),
        ir.forma("Neq", "nonequilibrium", "trabalho", "30s",
                 source_path="cpu_power", maintenance_deadline="2s"),
    )
    assert engine.retained_bytes["Ev"] <= vbl.ORCAMENTO_RETENCAO["event"]
    assert engine.retained_bytes["Eq"] <= vbl.ORCAMENTO_RETENCAO["equilibrium"]
    assert engine.retained_bytes["Neq"] <= vbl.ORCAMENTO_RETENCAO["nonequilibrium"]
    assert engine.labor_registry["Neq"] > 0
    engine.dissolve_form("Neq", fim="dissolve_rule")
    assert "Neq" not in engine.retained_bytes
    assert "Neq" not in engine.labor_registry  # 0 bytes de trabalho retidos
