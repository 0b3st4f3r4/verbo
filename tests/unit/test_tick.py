# -*- coding: utf-8 -*-
"""Ordem e precedência no tick (FORMAL §4.2/§4.5) e integridade do Caderno.

- regras avaliadas na ordem declarada, antes dos prazos;
- dissolve/subvert encurtam as regras seguintes (`review_short_circuit`),
  sem revogar atuações já despachadas;
- subvert não cancela as ações seguintes da mesma regra (§4.5);
- partilha igual da potência global P/N (§4.2);
- cadeia SHA-256 do Caderno detecta adulteração (AGENTS.md, AC).
"""

from __future__ import annotations

import hashlib
import json

import pytest

from fxp_sim import ir, loader


def _load(engine, *declarations, main=None):
    return loader.load(engine, ir.program(*declarations, main=main))


def test_rules_evaluated_in_declared_order(engine, ledger, sim):
    """Duas regras true ao mesmo tempo: a PRIMEIRA declarada age primeiro."""
    _load(
        engine,
        ir.form("Dupla", "event", "v", "30s"),
        ir.review(
            "Dupla",
            ir.rule("cpu_temp", ">", 10, "°C", ir.act_("Fan", 100)),
            ir.rule("cpu_temp", ">", 10, "°C", ir.act_("Fan", 150)),
        ),
    )
    sim.set_sensor("cpu_temp", 40.0)
    engine.tick()
    deliveries = [e["value"] for e in sim.delivered if e["actor"] == "Fan"]
    assert deliveries == [100, 150]  # ordem declarada preservada


def test_review_short_circuit_after_dissolution(engine, ledger, sim):
    """Regra 1 dissolve; regra 2 (que também dispararia) não é avaliada —
    e a atuação já despachada pela regra 1 não é revogada (FORMAL §4.2)."""
    _load(
        engine,
        ir.form("Curto", "event", "v", "30s"),
        ir.review(
            "Curto",
            ir.rule("cpu_temp", ">", 10, "°C", ir.act_("Fan", 100),
                    ir.action("dissolve")),
            ir.rule("cpu_temp", ">", 10, "°C", ir.act_("StatusLed", "red")),
        ),
    )
    sim.set_sensor("cpu_temp", 40.0)
    engine.tick()
    assert ledger.has("review_short_circuit", forma="Curto", regras_restantes=1)
    assert ledger.has("dissolve_rule", forma="Curto")
    assert any(e["actor"] == "Fan" for e in sim.delivered)
    assert not any(e["actor"] == "StatusLed" for e in sim.delivered)


def test_subvert_does_not_cancel_act_of_same_rule(engine, ledger, sim):
    """FORMAL §4.5 item 3: a action_list continua após o subvert — o `act`
    associado é enviado ao FXP."""
    _load(
        engine,
        ir.form("Trading", "nonequilibrium", "lucro", "30s",
                source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Trading", ir.rule("cpu_temp", ">", 85, "°C",
                                     ir.action("subvert"),
                                     ir.act_("CpuPowerCap", 50))),
    )
    sim.set_sensor("cpu_temp", 86.5)
    engine.tick()
    assert ledger.has("dissolve_subvert", forma="Trading")
    assert ledger.has("subvert_applied", forma="Trading",
                      novo_valor="poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente")
    assert any(e["actor"] == "CpuPowerCap" and e["value"] == 50
               for e in sim.delivered)
    assert "Trading" not in engine.forms  # dissolvida no mesmo tick
    assert engine.clock == 1  # ≤ 1 tick virtual


def test_dispatched_actuation_not_revoked_by_subvert(engine, ledger, sim):
    """`subvert` antes do `act` na lista: o act é enviado e a forma dissolve
    no mesmo tick (§4.5: sem revogar atuações já despachadas)."""
    _load(
        engine,
        ir.form("T", "nonequilibrium", "v", "30s", source_path="cpu_temp",
                maintenance_deadline="10s"),
        ir.review("T", ir.rule("cpu_temp", ">", 85, "°C",
                               ir.action("subvert"), ir.act_("StatusLed", "green"))),
    )
    sim.set_sensor("cpu_temp", 90.0)
    engine.tick()
    assert sim.actors["StatusLed"].current == "green"


def test_notify_shutdown_neither_dissolves_nor_interrupts(engine, ledger, sim):
    """FORMAL §4.6: notify_shutdown sinaliza desligamento de cargas
    secundárias; NÃO dissolve a forma e NÃO interrompe as ações seguintes
    da mesma regra."""
    _load(
        engine,
        ir.form("T", "nonequilibrium", "v", "30s", source_path="attention",
                maintenance_deadline="10s"),
        ir.review("T", ir.rule("attention", "<", 20, "%",
                               ir.action("notify_shutdown"),
                               ir.act_("StatusLed", "off"))),
    )
    sim.set_sensor("attention", 10.0)
    engine.tick()
    assert "T" in engine.forms            # forma permanece ativa
    assert not ledger.has("dissolve_rule")
    assert sim.actors["StatusLed"].current == "off"  # ação seguinte executada


def test_equal_share_of_global_power(engine, ledger):
    """FORMAL §4.2: cada forma registra P/N × duração do tick."""
    _load(
        engine,
        ir.form("A", "event", "a", "30s"),
        ir.form("B", "event", "b", "30s"),
    )
    engine.fxp.cpu_power = 100.0
    engine.tick()
    leaks = ledger.find("LEAK")
    by_form = {e["forma"]: e["watts"] for e in leaks}
    assert by_form["A"] == pytest.approx(50.0)
    assert by_form["B"] == pytest.approx(50.0)
    assert by_form["A"] + by_form["B"] == pytest.approx(100.0)


def test_sha256_chain_detects_tampering(vbl, ledger):
    ledger.event("INFO", "evento 1")
    ledger.event("INFO", "evento 2")
    assert ledger.verify_chain() is True
    # adulteração retroativa quebra a cadeia
    vbl.Caderno._events[0]["msg"] = "forjado"
    assert ledger.verify_chain() is False


def test_jsonl_export_reproduces_the_chain(vbl, ledger, tmp_path):
    ledger.event("INFO", "a")
    ledger.event("LEAK", "b", forma="X", joules=1.5)
    path = tmp_path / "log.jsonl"
    vbl.Caderno.export_jsonl(str(path))
    events = [json.loads(line) for line in path.read_text().splitlines()]
    assert [e["kind"] for e in events] == ["INFO", "LEAK"]
    # recomputar a cadeia do arquivo confere com a cabeça registrada
    head = "0" * 64
    for e in events:
        line = f"{e['seq']}\x1f{e['kind']}\x1f{e['msg']}"
        extra = {k: v for k, v in e.items()
                 if k not in ("seq", "kind", "msg", "hash")}
        if extra:
            line += "\x1f" + json.dumps(extra, sort_keys=True, ensure_ascii=False)
        head = hashlib.sha256((head + line).encode("utf-8")).hexdigest()
        assert head == e["hash"]
