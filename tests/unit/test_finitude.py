# -*- coding: utf-8 -*-
"""Asserções de Finitude (PLAN §1.2) — horizontes, prazos e manutenção.

Rastreabilidade: FORMAL §4.1 (conjugações, horizon absoluto, manutenção),
§4.2 (ordem no tick), AGENTS.md "Done" da Etapa 1.
"""

from __future__ import annotations

from fxp_sim import ir, loader


def _load(engine, *declarations, main=None):
    return loader.load(engine, ir.program(*declarations, main=main))


# ----------------------------------------------------------------------
# Horizonte de `event` (Asserções de Finitude — PLAN §1.2)
# ----------------------------------------------------------------------
def test_event_expires_at_horizon_with_typed_end(engine, ledger):
    _load(engine, ir.form("Piscada", "event", "impulso_curto", "3s"))
    engine.tick()  # t=1
    engine.tick()  # t=2 — ainda ativa
    assert "Piscada" in engine.forms
    engine.tick()  # t=3 — horizon esgota (>=)
    assert "Piscada" not in engine.forms
    assert ledger.has("dissolve_horizon", forma="Piscada")


def test_horizon_is_absolute_reclassification_does_not_renew(engine, ledger, sim):
    """FORMAL §4.1: horizon é contado desde a criação; reclassificar não o
    renova. NEQ (horizon 5s) vira EQ em t=1 e dissolve em t=5."""
    _load(
        engine,
        ir.form("Pensar", "nonequilibrium", "ideia", "5s",
                source_path="attention", maintenance_deadline="3s"),
        ir.review("Pensar",
                  ir.rule("attention", "<", 30, "%",
                          ir.action("reclassify_as_equilibrium"))),
    )
    sim.set_sensor("attention", 15.0)
    engine.tick()  # t=1: reclassifica para equilibrium (persistida)
    assert ledger.has("transicao", forma="Pensar", para="equilibrium")
    assert engine.forms["Pensar"].creation_time == 0.0  # não renovado
    sim.set_sensor("attention", 90.0)  # regra para de disparar
    for _ in range(3):  # t=2,3,4
        engine.tick()
    assert "Pensar" in engine.forms  # t=4: ainda viva
    engine.tick()  # t=5: horizon original esgota
    assert "Pensar" not in engine.forms
    assert ledger.has("dissolve_horizon", forma="Pensar")


def test_equilibrium_also_expires_by_horizon(engine, ledger):
    """Matriz de transições (FORMAL §4.1): EQ -> DIS por horizon."""
    _load(engine, ir.form("Registro", "equilibrium", "doc", "2s"))
    engine.tick()
    assert "Registro" in engine.forms  # t=1: 1 >= 2? não
    engine.tick()  # t=2: 2 >= 2 — horizon esgota no próprio limite
    assert "Registro" not in engine.forms
    assert ledger.has("dissolve_horizon", forma="Registro")


# ----------------------------------------------------------------------
# Manutenção de `nonequilibrium` (FORMAL §4.1)
# ----------------------------------------------------------------------
def test_nonequilibrium_with_active_rule_has_implicit_maintenance(engine, ledger):
    """FORMAL §4.1(ii): manutenção implícita a cada tick enquanto houver
    regra de revisão ativa — a forma sobrevive além do deadline sem keep()."""
    _load(
        engine,
        ir.form("Vigilia", "nonequilibrium", "trabalho", "30s",
                source_path="attention", maintenance_deadline="2s"),
        ir.review("Vigilia", ir.rule("attention", "<", 5, "%", ir.action("dissolve"))),
    )
    for _ in range(6):  # attention = 100 -> regra nunca dispara
        engine.tick()
    assert "Vigilia" in engine.forms
    assert not ledger.has("collapse_maintenance")


def test_nonequilibrium_without_maintenance_collapses_at_first_deadline(engine, ledger):
    """Sem regra ativa e sem keep(): colapsa no primeiro vencimento do
    maintenance_deadline ('exceder' = estritamente maior, FORMAL §4.1)."""
    _load(
        engine,
        ir.form("Solo", "nonequilibrium", "sem_vigilia", "30s",
                source_path="cpu_power", maintenance_deadline="2s"),
    )
    engine.tick()  # t=1: 1 > 2? não
    engine.tick()  # t=2: 2 > 2? não (limite estrito)
    assert "Solo" in engine.forms
    engine.tick()  # t=3: 3 > 2? sim — colapso
    assert "Solo" not in engine.forms
    assert ledger.has("collapse_maintenance", forma="Solo")


def test_manual_keep_renews_the_deadline(engine, ledger):
    """FORMAL §4.1(i): keep(forma) explícito renova o prazo de manutenção."""
    interpreter = _load(
        engine,
        ir.form("Solo", "nonequilibrium", "mantido", "30s",
                source_path="cpu_power", maintenance_deadline="2s"),
        main=ir.main_block(ir.every("1s", ir.keep_("Solo"))),
    )
    for _ in range(6):
        interpreter.run_due()
        engine.tick()
    assert "Solo" in engine.forms
    assert not ledger.has("collapse_maintenance")


# ----------------------------------------------------------------------
# Fim único e ordem no tick (FORMAL §4.2)
# ----------------------------------------------------------------------
def test_form_ends_once_and_only_once_per_tick(engine, ledger, sim):
    """Regra que dissolve no mesmo tick do vencimento do horizon: a forma
    termina UMA vez, com o fim da regra (avaliada antes dos prazos)."""
    _load(
        engine,
        ir.form("Ciclo", "nonequilibrium", "lucro", "3s",
                source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Ciclo", ir.rule("cpu_temp", ">", 85, "°C",
                                   ir.action("dissolve"))),
    )
    sim.set_sensor("cpu_temp", 90.0)
    engine.tick()  # t=3 não chegou; regra dispara primeiro
    assert "Ciclo" not in engine.forms
    assert ledger.kinds().count("dissolve_rule") == 1
    assert not ledger.has("dissolve_horizon")


# ----------------------------------------------------------------------
# Contadores de retenção do runtime (proxy de heap — PLAN §1.2 iii)
# ----------------------------------------------------------------------
def test_retention_counters_within_budgets(engine, ledger, vbl):
    _load(
        engine,
        ir.form("Ev", "event", "curto", "3s"),
        ir.form("Eq", "equilibrium", "doc", "30s", cost_bytes=128),
        ir.form("Neq", "nonequilibrium", "trabalho", "30s",
                source_path="cpu_power", maintenance_deadline="2s"),
    )
    assert engine.retained_bytes["Ev"] <= vbl.ORCAMENTO_RETENCAO["event"]
    assert engine.retained_bytes["Eq"] <= vbl.ORCAMENTO_RETENCAO["equilibrium"]
    assert engine.retained_bytes["Neq"] <= vbl.ORCAMENTO_RETENCAO["nonequilibrium"]
    assert engine.labor_registry["Neq"] > 0
    engine.dissolve_form("Neq", fim="dissolve_rule")
    assert "Neq" not in engine.retained_bytes
    assert "Neq" not in engine.labor_registry  # 0 bytes de trabalho retidos
