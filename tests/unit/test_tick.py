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


def _carregar(engine, *declaracoes, main=None):
    return loader.carregar(engine, ir.programa(*declaracoes, main=main))


def test_regras_sao_avaliadas_na_ordem_declarada(engine, cad, sim):
    """Duas regras true ao mesmo tempo: a PRIMEIRA declarada age primeiro."""
    _carregar(
        engine,
        ir.forma("Dupla", "event", "v", "30s"),
        ir.review(
            "Dupla",
            ir.regra("cpu_temp", ">", 10, "°C", ir.act_("Ventoinha", 100)),
            ir.regra("cpu_temp", ">", 10, "°C", ir.act_("Ventoinha", 150)),
        ),
    )
    sim.set_sensor("cpu_temp", 40.0)
    engine.tick()
    entregas = [e["valor"] for e in sim.entregues if e["ator"] == "Ventoinha"]
    assert entregas == [100, 150]  # ordem declarada preservada


def test_review_short_circuit_apos_dissolucao(engine, cad, sim):
    """Regra 1 dissolve; regra 2 (que também dispararia) não é avaliada —
    e a atuação já despachada pela regra 1 não é revogada (FORMAL §4.2)."""
    _carregar(
        engine,
        ir.forma("Curto", "event", "v", "30s"),
        ir.review(
            "Curto",
            ir.regra("cpu_temp", ">", 10, "°C", ir.act_("Ventoinha", 100),
                     ir.acao("dissolve")),
            ir.regra("cpu_temp", ">", 10, "°C", ir.act_("LedIndicador", "vermelho")),
        ),
    )
    sim.set_sensor("cpu_temp", 40.0)
    engine.tick()
    assert cad.tem("review_short_circuit", forma="Curto", regras_restantes=1)
    assert cad.tem("dissolve_rule", forma="Curto")
    assert any(e["ator"] == "Ventoinha" for e in sim.entregues)
    assert not any(e["ator"] == "LedIndicador" for e in sim.entregues)


def test_subvert_nao_cancela_act_da_mesma_regra(engine, cad, sim):
    """FORMAL §4.5 item 3: a action_list continua após o subvert — o `act`
    associado é enviado ao FXP."""
    _carregar(
        engine,
        ir.forma("Trading", "nonequilibrium", "lucro", "30s",
                 source_path="cpu_temp", maintenance_deadline="10s"),
        ir.review("Trading", ir.regra("cpu_temp", ">", 85, "°C",
                                      ir.acao("subvert"),
                                      ir.act_("CpuPowerCap", 50))),
    )
    sim.set_sensor("cpu_temp", 86.5)
    engine.tick()
    assert cad.tem("dissolve_subvert", forma="Trading")
    assert cad.tem("subvert_aplicado", forma="Trading",
                   novo_valor="poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente")
    assert any(e["ator"] == "CpuPowerCap" and e["valor"] == 50
               for e in sim.entregues)
    assert "Trading" not in engine.forms  # dissolvida no mesmo tick
    assert engine.clock == 1  # ≤ 1 tick virtual


def test_atuacao_despachada_nao_e_revogada_pelo_subvert(engine, cad, sim):
    """`subvert` antes do `act` na lista: o act é enviado e a forma dissolve
    no mesmo tick (§4.5: sem revogar atuações já despachadas)."""
    _carregar(
        engine,
        ir.forma("T", "nonequilibrium", "v", "30s", source_path="cpu_temp",
                 maintenance_deadline="10s"),
        ir.review("T", ir.regra("cpu_temp", ">", 85, "°C",
                                ir.acao("subvert"), ir.act_("LedIndicador", "verde"))),
    )
    sim.set_sensor("cpu_temp", 90.0)
    engine.tick()
    assert sim.atores["LedIndicador"].atual == "verde"


def test_notify_shutdown_nao_dissolve_nem_interrompe(engine, cad, sim):
    """FORMAL §4.6: notify_shutdown sinaliza desligamento de cargas
    secundárias; NÃO dissolve a forma e NÃO interrompe as ações seguintes
    da mesma regra."""
    _carregar(
        engine,
        ir.forma("T", "nonequilibrium", "v", "30s", source_path="attention",
                 maintenance_deadline="10s"),
        ir.review("T", ir.regra("attention", "<", 20, "%",
                                ir.acao("notify_shutdown"),
                                ir.act_("LedIndicador", "apagado"))),
    )
    sim.set_sensor("attention", 10.0)
    engine.tick()
    assert "T" in engine.forms            # forma permanece ativa
    assert not cad.tem("dissolve_rule")
    assert sim.atores["LedIndicador"].atual == "apagado"  # ação seguinte executada


def test_partilha_igual_da_potencia_global(engine, cad):
    """FORMAL §4.2: cada forma registra P/N × duração do tick."""
    _carregar(
        engine,
        ir.forma("A", "event", "a", "30s"),
        ir.forma("B", "event", "b", "30s"),
    )
    engine.fxp.cpu_power = 100.0
    engine.tick()
    vazamentos = cad.buscar("VAZAMENTO")
    por_forma = {e["forma"]: e["watts"] for e in vazamentos}
    assert por_forma["A"] == pytest.approx(50.0)
    assert por_forma["B"] == pytest.approx(50.0)
    assert por_forma["A"] + por_forma["B"] == pytest.approx(100.0)


def test_cadeia_sha256_detecta_adulteracao(vbl, cad):
    cad.event("INFO", "evento 1")
    cad.event("INFO", "evento 2")
    assert cad.verify_chain() is True
    # adulteração retroativa quebra a cadeia
    vbl.Caderno._events[0]["msg"] = "forjado"
    assert cad.verify_chain() is False


def test_exportacao_jsonl_reproduz_a_cadeia(vbl, cad, tmp_path):
    cad.event("INFO", "a")
    cad.event("VAZAMENTO", "b", forma="X", joules=1.5)
    caminho = tmp_path / "log.jsonl"
    vbl.Caderno.export_jsonl(str(caminho))
    eventos = [json.loads(linha) for linha in caminho.read_text().splitlines()]
    assert [e["kind"] for e in eventos] == ["INFO", "VAZAMENTO"]
    # recomputar a cadeia do arquivo confere com a cabeça registrada
    head = "0" * 64
    for e in eventos:
        line = f"{e['seq']}\x1f{e['kind']}\x1f{e['msg']}"
        extra = {k: v for k, v in e.items()
                 if k not in ("seq", "kind", "msg", "hash")}
        if extra:
            line += "\x1f" + json.dumps(extra, sort_keys=True, ensure_ascii=False)
        head = hashlib.sha256((head + line).encode("utf-8")).hexdigest()
        assert head == e["hash"]
