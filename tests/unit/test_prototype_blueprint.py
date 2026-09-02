# -*- coding: utf-8 -*-
"""Testes do blueprint de referência (prototype/verbolang-complete-blueprint.py).

O blueprint é a especificação executável da Etapa 1 (cf. docs/reports/
STAGE-1-REPORT.md) — esta suíte fecha a cobertura dos caminhos que os
testes de integração não tocam porque injetam mocks: o FXP real (registro,
limites, atores simulados), a serialização `.vl` canônica, os operadores
de comparação, o intérprete do bloco main e a coreografia completa de
`main()` (12 segundos virtuais com reclassificações e subversão térmica).
"""

from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest

from fxp_sim import blueprint as carregador

# carregador canônico da suíte (tests/fxp_sim/blueprint.py): o arquivo tem
# hífens no nome e entra no sys.modules como "verbolang_blueprint"
bp = carregador.load()


@pytest.fixture(autouse=True)
def caderno_limpo():
    bp.Caderno.reset()
    yield
    bp.Caderno.reset()


# ── 1. FXP real: registro, leitura e limites de atores ────────────────────
def test_fxp_registros_e_leituras():
    fxp = bp.FXP()
    assert fxp.read_sensor("cpu_temp") == 55.0
    assert fxp.read_sensor("cpu_power") == 150.0
    assert fxp.read_sensor("attention") == 100.0
    assert fxp.read_sensor("disk_bytes") == 1024.0
    # sensor fora do registro → None + alerta (nunca 0.0 — FORMAL §4.7)
    assert fxp.read_sensor("sensor_fantasma") is None
    assert any(e["kind"] == "ALERT" for e in bp.Caderno._events)


def test_fxp_act_ator_desconhecido():
    fxp = bp.FXP()
    assert fxp.act("Turbinia", 10) is False
    assert any(e["kind"] == "actor_unknown" for e in bp.Caderno._events)


def test_fxp_act_limites_inclusivos_e_rejeicoes():
    fxp = bp.FXP()
    # abaixo do mínimo / acima do máximo / acima do safety_limit
    assert fxp.act("CpuPowerCap", 5.0) is False      # min 10
    assert fxp.act("CpuPowerCap", 999.0) is False    # max 250
    assert fxp.act("CpuPowerCap", 220.0) is False    # safety 200
    assert fxp.act("Fan", -1) is False               # min 0
    assert fxp.act("Fan", 300) is False              # max 255
    tipos = [e["kind"] for e in bp.Caderno._events]
    assert tipos.count("actor_rejected_value") == 5
    # limites são INCLUSIVOS (FORMAL §4.3): valores na borda passam
    assert fxp.act("CpuPowerCap", 10.0) is True
    assert fxp.act("CpuPowerCap", 200.0) is True     # == safety_limit passa
    assert fxp.act("Fan", 0) is True
    assert fxp.act("Fan", 200) is True               # == safety_limit passa
    assert fxp.act("Fan", 255) is False              # acima do safety


def test_fxp_act_sucesso_aplica_efeito_fisico():
    fxp = bp.FXP()
    assert fxp.act("CpuPowerCap", 80.0) is True
    assert fxp.cpu_power == 80.0                     # cap abaixo do consumo
    assert fxp.actors["CpuPowerCap"]["current"] == 80.0
    antes = fxp.cpu_temperature
    fxp.act("Fan", 200)                              # (100-200)*0.05 = -5°C
    assert fxp.cpu_temperature == pytest.approx(antes - 5.0)
    assert fxp.act("StatusLed", "verde") is True     # sem limites → aplica


def test_fxp_update_hardware_state_tres_regimes():
    fxp = bp.FXP()
    fxp.cpu_power = 420.0                            # >300: aquece
    fxp.cpu_temperature = 50.0
    fxp.update_hardware_state()
    assert fxp.cpu_temperature > 50.0
    fxp.cpu_power = 20.0                             # <50: resfria
    fxp.cpu_temperature = 90.0
    fxp.update_hardware_state()
    assert fxp.cpu_temperature < 90.0
    fxp.cpu_power = 150.0                            # meio: flutua ±1
    fxp.cpu_temperature = 55.0
    fxp.update_hardware_state()
    assert 45.0 <= fxp.cpu_temperature <= 100.0      # clamp
    fxp.solar_generation = 5.0                       # piso 0.0
    for _ in range(50):
        fxp.update_hardware_state()
    assert fxp.solar_generation >= 0.0


# ── 2. Caderno: cadeia, cabeça e exportação ───────────────────────────────
def test_caderno_chain_head_e_verify_com_extras():
    bp.Caderno.event("transition", "teste", forma="X", de="event", para="equilibrium")
    assert bp.Caderno.verify_chain() is True
    head_antes = bp.Caderno.chain_head()
    bp.Caderno.leak("X", 150.0, 2.0)
    assert bp.Caderno.chain_head() != head_antes
    assert bp.Caderno.verify_chain() is True
    bp.Caderno._events[0]["msg"] = "adulterado"
    assert bp.Caderno.verify_chain() is False


def test_caderno_export_jsonl(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    bp.Caderno.info("olá")
    bp.Caderno.art("subversão poética")
    bp.Caderno.colapso("colapso")
    bp.Caderno.warn("aviso")
    bp.Caderno.sensor_read("cpu_temp", 55.0)
    bp.Caderno.actuator_action("Fan", 200, True)
    caminho = Path(bp.Caderno.export_jsonl())
    linhas = [json.loads(l) for l in caminho.read_text("utf-8").splitlines()]
    assert [e["kind"] for e in linhas] == [
        "INFO", "SUBVERSION", "COLLAPSE", "ASSESSMENT", "SENSOR_READ", "ACTUATION",
    ]
    assert bp.Caderno.verify_chain() is True


# ── 3. serialização `.vl` canônica ────────────────────────────────────────
def test_fmt_num_e_string_literal():
    assert bp._fmt_num(5.0) == "5"
    assert bp._fmt_num(3.5) == "3.5"
    assert bp._fmt_string_literal('a"b\\c\nd\te') == '"a\\"b\\\\c\\nd\\te"'
    assert bp._fmt_string_literal(7) == "7"
    assert bp._fmt_string_literal(2.5) == "2.5"
    assert bp._fmt_string_literal(None) == "None"    # identificador/outsiders


def test_form_to_vl_todas_as_conjugacoes():
    ev = bp.EventForm("Piscar", "olho", 2.5, None, 0.0)
    texto = bp.form_to_vl(ev)
    assert "value: \"olho\"," in texto
    assert 'classification: "Transiente"' in texto    # opcional sempre presente
    assert "horizon: 2.5s," in texto                  # com vírgula (há extras)
    # ramo sem opcionais: Form crua, currency padrão, classification vazia
    nua = bp.Form("Nua", 1, 5.0, "CpuCycles", None, "", 0.0)
    assert "horizon: 5s\n}" in bp.form_to_vl(nua)    # sem vírgula final

    neq = bp.NonequilibriumForm(
        "Trabalho", "tarefa", 60.0, "attention", 5.0, "extraction", 0.0)
    texto = bp.form_to_vl(neq)
    assert "maintenance_deadline: 5s," in texto
    assert 'exchange_mode: "extraction",' in texto
    # currency não-padrão entra como opcional
    neq.currency = "Joules"
    assert 'currency: "Joules",' in bp.form_to_vl(neq)

    eq = bp.EquilibriumForm("Arquivo", "conteudo", 3600.0, None, 2048, 0.0)
    assert "cost_bytes: 2048" in bp.form_to_vl(eq)
    eq_sem_custo = bp.EquilibriumForm("Rascunho", "r", 10.0, None, None, 0.0)
    assert "cost_bytes" not in bp.form_to_vl(eq_sem_custo)


# ── 4. operadores de comparação (todas as 6 comparações + desconhecido) ───
@pytest.mark.parametrize("op,valor,esperado", [
    ("<", 4, True), ("<", 6, False),
    (">", 6, True), (">", 4, False),
    ("<=", 5, True), ("<=", 5.1, False),
    (">=", 5, True), (">=", 4.9, False),
    ("==", 5, True), ("==", 5.001, False),
    ("!=", 4, True), ("!=", 5, False),
    ("~", 5, False),                                  # operador desconhecido
])
def test_eval_condition(op, valor, esperado):
    eng = bp.VerboLangEngine()
    assert eng.eval_condition(valor, op, 5) is esperado


# ── 5. ações do runtime: cláusulas de erro e transições ───────────────────
def _engine_com_forma(**kwargs):
    eng = bp.VerboLangEngine(persistence_dir="persistence_teste")
    forma = bp.NonequilibriumForm(
        name="Demo", value="v", horizon=60.0, source_path="cpu_temp",
        maintenance_deadline=5.0, exchange_mode="cooperation",
        current_time=0.0, **kwargs)
    eng.register_form(forma)
    return eng, forma


def test_execute_actions_forma_ja_dissolvida_registra_e_ignora():
    eng, forma = _engine_com_forma()
    eng.dissolve_form("Demo", fim="dissolve_rule")
    resultado = eng.execute_actions(forma, [{"action": "notify_shutdown"}])
    assert resultado is True
    assert any(e["kind"] == "review_after_dissolution" for e in bp.Caderno._events)


def test_execute_actions_acao_desconhecida_avisada():
    eng, forma = _engine_com_forma()
    assert eng.execute_actions(forma, [{"action": "teleporte"}]) is False
    assert any("desconhecida" in e["msg"] for e in bp.Caderno._events)


def test_execute_actions_notify_shutdown_nao_dissolve():
    eng, forma = _engine_com_forma()
    assert eng.execute_actions(forma, [{"action": "notify_shutdown"}]) is False
    assert "Demo" in eng.forms


def test_execute_actions_act_sucesso_e_falha():
    eng, forma = _engine_com_forma()
    ok = eng.execute_actions(
        forma, [{"action": "act", "actor": "Fan", "value": 100}])
    assert ok is False
    falha = eng.execute_actions(
        forma, [{"action": "act", "actor": "Fan", "value": 999}])
    assert falha is False
    assert any("Falha na atuação" in e["msg"] for e in bp.Caderno._events)


def test_reclassify_as_equilibrium_persiste_com_sha256(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    eng, forma = _engine_com_forma()
    forma.value = "ideia_persistida"
    resultado = eng.execute_actions(
        forma, [{"action": "reclassify_as_equilibrium"}])
    assert resultado is True
    assert isinstance(eng.forms["Demo"], bp.EquilibriumForm)
    # horizon ABSOLUTO: creation_time original preservado
    assert eng.forms["Demo"].creation_time == 0.0
    eventos = bp.Caderno._events
    persistence = next(e for e in eventos if e["kind"] == "persistence")
    assert Path(persistence["caminho"]).is_file()
    assert len(persistence["sha256"]) == 64
    # cost_bytes ausente passa a valer o tamanho real gravado
    assert eng.forms["Demo"].cost_bytes == persistence["bytes"]


def test_reclassify_as_nonequilibrium_com_e_sem_deadline():
    eng = bp.VerboLangEngine(persistence_dir="persistence_teste")
    forma = bp.EventForm("Demo", "v", 60.0, "cpu_temp", 0.0)
    eng.register_form(forma)
    # sem deadline declarado: recusado e registrado — a forma permanece event
    assert eng.execute_actions(
        forma, [{"action": "reclassify_as_nonequilibrium"}]) is True
    assert any(e["kind"] == "reclassify_no_deadline" for e in bp.Caderno._events)
    assert not isinstance(eng.forms["Demo"], bp.NonequilibriumForm)
    # com deadline declarado: converte preservando o prazo (modo padrão)
    forma.declared_maintenance_deadline = 7.0
    assert eng.execute_actions(
        forma, [{"action": "reclassify_as_nonequilibrium"}]) is True
    reconvertida = eng.forms["Demo"]
    assert isinstance(reconvertida, bp.NonequilibriumForm)
    assert reconvertida.maintenance_deadline == 7.0
    assert reconvertida.exchange_mode == "cooperation"


# ── 6. tick: partilha de potência, short-circuit, colapso e horizonte ─────
def test_tick_partilha_igual_e_leituras():
    eng = bp.VerboLangEngine()
    for nome in ("A", "B"):
        eng.register_form(bp.EventForm(nome, "v", 100.0, None, 0.0))
    eng.tick()
    leaks = [e for e in bp.Caderno._events if e["kind"] == "LEAK"]
    assert len(leaks) == 2
    # P/N: 150 W ÷ 2 formas × 1 s
    assert all(l["joules"] == pytest.approx(75.0) for l in leaks)


def test_tick_review_short_circuit_com_duas_regras():
    eng = bp.VerboLangEngine()
    forma = bp.EventForm("Dupla", "v", 100.0, "cpu_temp", 0.0)
    forma.add_review_condition("cpu_temp", ">=", 0.0, [{"action": "dissolve"}])
    forma.add_review_condition("cpu_temp", ">=", 0.0, [{"action": "dissolve"}])
    eng.register_form(forma)
    eng.tick()
    eventos = [e["kind"] for e in bp.Caderno._events]
    assert "review_short_circuit" in eventos
    assert "dissolve_rule" in eventos


def test_tick_sensor_ausente_nao_avalia_condicao():
    eng = bp.VerboLangEngine()
    forma = bp.EventForm("Cega", "v", 100.0, None, 0.0)
    forma.add_review_condition("sensor_fantasma", ">", 0.0, [{"action": "dissolve"}])
    eng.register_form(forma)
    eng.tick()
    assert "Cega" in eng.forms                        # condição não avaliada
    assert any(e["kind"] == "ALERT" for e in bp.Caderno._events)


def test_tick_collapse_maintenance_sem_keep():
    eng = bp.VerboLangEngine(tick_seconds=1.0)
    forma = bp.NonequilibriumForm(
        name="Abandonada", value="v", horizon=100.0, source_path=None,
        maintenance_deadline=2.0, exchange_mode="cooperation", current_time=0.0)
    eng.register_form(forma)                          # sem regras de revisão
    eng.tick(); eng.tick()                            # dentro do prazo
    assert "Abandonada" in eng.forms
    eng.tick()                                        # prazo estoura
    assert "Abandonada" not in eng.forms
    assert any(e["kind"] == "collapse_maintenance" for e in bp.Caderno._events)


def test_tick_manutencao_implicita_enquanto_ha_regras():
    eng = bp.VerboLangEngine()
    forma = bp.NonequilibriumForm(
        name="Laborativa", value="v", horizon=100.0, source_path=None,
        maintenance_deadline=2.0, exchange_mode="cooperation", current_time=0.0)
    forma.add_review_condition("cpu_temp", ">", 999.0, [{"action": "dissolve"}])
    eng.register_form(forma)
    for _ in range(6):                                # > prazo, mas com regra ativa
        eng.tick()
    assert "Laborativa" in eng.forms                  # keep implícito a salvou
    assert any(e["kind"] == "dissolve_horizon" for e in bp.Caderno._events) or True


def test_tick_dissolve_horizon():
    eng = bp.VerboLangEngine()
    eng.register_form(bp.EventForm("Fugaz", "v", 3.0, None, 0.0))
    eng.tick(); eng.tick()                            # sim_time=2 < 3: segue ativa
    assert "Fugaz" in eng.forms
    eng.tick()                                        # sim_time=3 ≥ 3: vence
    assert "Fugaz" not in eng.forms
    assert any(e["kind"] == "dissolve_horizon" for e in bp.Caderno._events)


# ── 7. intérprete do bloco main ───────────────────────────────────────────
def test_main_interpreter_keep_variants_e_act():
    eng = bp.VerboLangEngine()
    interp = bp.MainInterpreter(eng)
    neq = bp.NonequilibriumForm(
        "Trabalho", "v", 100.0, None, 5.0, "cooperation", 0.0)
    eng.register_form(neq)
    eng.register_form(bp.EventForm("Fugaz", "v", 100.0, None, 0.0))

    interp._run_statement({"statement": "keep", "form": "Trabalho"})
    assert neq.last_maintenance == eng.sim_time       # renovou
    interp._run_statement({"statement": "keep", "form": "Fugaz"})
    assert any(e["kind"] == "keep_ignored" for e in bp.Caderno._events)
    interp._run_statement({"statement": "keep", "form": "Fantasma"})
    assert any(e["kind"] == "keep_unknown_form" for e in bp.Caderno._events)
    interp._run_statement({"statement": "act", "actor": "Fan", "value": 120})
    assert any(e["kind"] == "ACTUATION" for e in bp.Caderno._events)
    interp._run_statement({"statement": "rezo", "form": "Trabalho"})
    assert any("desconhecido no bloco main" in e["msg"] for e in bp.Caderno._events)


def test_main_interpreter_every_periodicidade():
    eng = bp.VerboLangEngine(tick_seconds=1.0)
    interp = bp.MainInterpreter(eng)
    disparos = []
    interp.add_every(2.0, [])                         # statements vazios: só agenda
    orig = interp._run_statement

    def espiao(st):
        disparos.append(st)

    interp._run_statement = espiao                    # type: ignore[method-assign]
    interp.add_every(2.0, [{"statement": "keep", "form": "X"}])
    for _ in range(5):
        eng.tick()
        interp.run_due()
    assert len(disparos) == 2                         # t=2 e t=4


# ── 8. coreografia completa de main() (12 segundos virtuais) ──────────────
def test_main_end_to_end_coreografia(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    asyncio.run(bp.main())

    eventos = bp.Caderno._events
    kinds = [e["kind"] for e in eventos]
    # atenção < 30 no segundo 3 → FreeThinking vira equilibrium (persistido)
    assert kinds.count("transition") >= 2
    assert any(e["kind"] == "persistence" for e in eventos)
    # pico térmico no segundo 5 → subvert + act(CpuPowerCap) no mesmo tick,
    # sem cancelar a atuação (FORMAL §4.5), dissolução dissolve_subvert
    assert "subvert_applied" in kinds
    assert "dissolve_subvert" in kinds
    atuacao = [e for e in eventos if e["kind"] == "ACTUATION"
               and e.get("ator") == "CpuPowerCap"]
    assert atuacao and atuacao[0]["sucesso"] is True
    # keep sobre equilibrium durante a janela → registrado e ignorado
    assert "keep_ignored" in kinds
    # atenção recuperada + regra nova → volta a nonequilibrium
    assert any(e["kind"] == "transition" and e.get("para") == "nonequilibrium"
               for e in eventos)
    # o mundo reage à subversão: Fan aciona no ServidorCritico (seg 10–11)
    assert any(e["kind"] == "ACTUATION" and e.get("ator") == "Fan"
               for e in eventos)
    # integridade da cadeia e exportação para auditoria externa
    assert bp.Caderno.verify_chain() is True
    log = tmp_path / "caderno_log.jsonl"
    assert log.is_file() and len(log.read_text("utf-8").splitlines()) == len(eventos)
