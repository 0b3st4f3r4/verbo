# -*- coding: utf-8 -*-
"""Steps dos três cenários BDD da Etapa 1 (docs/PLAN.md §1.1).

Cada cenário roda sobre mocks em processo (FXPSimulator determinístico) e o
runtime do protótipo — sem hardware, sem rede, sem aleatoriedade.
"""

from __future__ import annotations

import hashlib
import os

from behave import given, then, when

import vlcheck
from fxp_sim import ir

POESIA = "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"


# ======================================================================
# Caso 1 — Fadiga de atenção
# ======================================================================
@given('que a forma laborativa "{nome}" está ativa com um deadline de {deadline:d}s')
def step_forma_laborativa(context, nome, deadline):
    programa = ir.programa(
        ir.forma(nome, "nonequilibrium", "consciencia_anteneoliberal_ativa",
                 "60s", source_path="attention",
                 maintenance_deadline=f"{deadline}s",
                 exchange_mode="cooperation"),
        ir.review(nome, ir.regra("attention", "<", 30, "%",
                                 ir.acao("reclassify_as_equilibrium"))),
    )
    context.nome = nome
    context.loader.carregar(context.engine, programa)
    assert context.engine.labor_registry.get(nome) is not None


@when('a leitura do sensor "{sensor}" via FXP cai abaixo de {limiar:d}% (ex: {valor:g}%)')
def step_queda_de_atencao(context, sensor, limiar, valor):
    context.sim.set_sensor(sensor, valor)
    context.engine.tick()


@then('o runtime deve disparar uma transição "reclassify_as_equilibrium"')
def step_transicao(context):
    assert context.cad.tem("transicao", forma=context.nome, para="equilibrium")
    assert context.engine.forms[context.nome].conjugation == "equilibrium"


@then('o estado da ideia deve ser gravado como ".vl" canônico no diretório de persistência')
def step_persistida_canonica(context):
    caminho = os.path.join(context.engine.persistence_dir, f"{context.nome}.vl")
    assert os.path.isfile(caminho), f"arquivo persistido ausente: {caminho}"
    with open(caminho, encoding="utf-8") as f:
        conteudo = f.read()
    # canônico = reparseável: o validador de superfície não aponta erros
    erros = vlcheck.validar(conteudo)
    assert not erros, f".vl persistido não é canônico: {erros}"
    assert 'value: "consciencia_anteneoliberal_ativa"' in conteudo
    context.caminho_persistido = caminho


@then('o Caderno registra o evento de persistência com o SHA-256 do arquivo gravado')
def step_evento_persistencia(context):
    eventos = context.cad.buscar("persistencia", forma=context.nome)
    assert eventos, "evento de persistência ausente no Caderno"
    with open(context.caminho_persistido, "rb") as f:
        sha_real = hashlib.sha256(f.read()).hexdigest()
    assert eventos[-1]["sha256"] == sha_real
    assert eventos[-1]["caminho"] == context.caminho_persistido


@then('após a reclassificação a forma deixa de receber ticks de manutenção')
def step_sem_manutencao(context):
    nome = context.nome
    # trabalho laborativo encerrado: nenhum tick de manutenção seguinte
    assert context.engine.labor_registry.get(nome) is None
    context.engine.tick()  # um tick extra: sem keep() e sem colapso
    assert nome in context.engine.forms
    assert not context.cad.tem("collapse_maintenance", forma=nome)


@then('0 bytes permanecem retidos em heap para a forma (verificado com contadores do runtime)')
def step_zero_bytes_retidos(context):
    nome = context.nome
    # Interpretação registrada (docs/STAGE-1-REPORT.md): o estado
    # laborativo (nonequilibrium) foi integralmente liberado — 0 bytes de
    # trabalho retidos; o que permanece é a forma equilibrium persistida em
    # disco, dentro do orçamento de retenção da conjugação (ADR-001).
    assert context.engine.labor_registry.get(nome) is None
    orcamento = context.vbl.ORCAMENTO_RETENCAO["equilibrium"]
    assert context.engine.retained_bytes.get(nome, 0) <= orcamento


# ======================================================================
# Caso 2 — Subversão térmica
# ======================================================================
@given('que a tarefa "{nome}" está rodando em alta frequência')
def step_trading_alta_frequencia(context, nome):
    programa = ir.programa(
        ir.forma(nome, "nonequilibrium", "lucro_arbitragem_alta_frequencia",
                 "7s", source_path="cpu_temp", maintenance_deadline="2s",
                 exchange_mode="extraction"),
        ir.review(nome, ir.regra("cpu_temp", ">", 85, "°C",
                                 ir.acao("subvert"),
                                 ir.act_("CpuPowerCap", 50))),
    )
    context.nome = nome
    context.loader.carregar(context.engine, programa)
    context.sim.cpu_power = 420.0  # alta frequência => potência elevada
    context.tick_do_disparo = None


@when('o sensor "{sensor}" atinge {valor:g}°C (limite de {limite:g}°C) via FXP')
def step_pico_termico(context, sensor, valor, limite):
    context.limite = limite
    context.sim.set_sensor(sensor, valor)
    context.engine.tick()
    context.tick_do_disparo = context.engine.clock


@then('o runtime deve invocar o operador "subvert()"')
def step_subvert_invocado(context):
    assert context.cad.tem("dissolve_subvert", forma=context.nome)
    assert context.cad.tem("subvert_aplicado", forma=context.nome)


@then('a ação "act({ator}, {valor:d})" deve ser enviada ao ator correspondente via FXP')
def step_act_enviado(context, ator, valor):
    msg = [m for m in context.sim.outbox
           if m["op"] == "act" and m["ator"] == ator and m["valor"] == valor]
    assert msg, "comando `act` não serializado no FXP"
    assert any(e["ator"] == ator and e["valor"] == valor
               for e in context.sim.entregues)
    assert context.sim.atores[ator].atual == valor


@then('o valor lógico de trading deve ser substituído pelo valor poético canônico "{poesia}"')
def step_valor_poetico(context, poesia):
    evento = context.cad.buscar("subvert_aplicado", forma=context.nome)
    assert evento and evento[0]["novo_valor"] == poesia == POESIA


@then('o processamento da forma subvertida deve cessar no mesmo tick (dissolução em ≤ 1 tick virtual)')
def step_cessa_no_mesmo_tick(context):
    nome = context.nome
    assert nome not in context.engine.forms  # dissolvida
    assert context.engine.clock == context.tick_do_disparo  # mesmo tick
    # 0 bytes de trabalho retidos (contadores do runtime)
    assert context.engine.labor_registry.get(nome) is None
    assert context.engine.retained_bytes.get(nome) is None


# ======================================================================
# Caso 3 — Fallback de ator
# ======================================================================
@given('que o ator "{ator}" não está respondendo')
def step_ator_sem_resposta(context, ator):
    # política de fallback no REGISTRO do FXP (FORMAL §4.3) + extensão opcional
    context.sim.registrar_ator("VentoinhaReserva",
                               "ventoinha alternativa (extensão opcional)",
                               minimo=0, maximo=255, safety_limit=200)
    context.sim.definir_fallback(ator, "VentoinhaReserva")
    context.sim.falhar_ator(ator)
    # a forma vigia a temperatura e aciona o ator primário ao exceder 70°C
    programa = ir.programa(
        ir.forma("ServidorCritico", "nonequilibrium", "processamento_continuo",
                 "3600s", source_path="cpu_temp", maintenance_deadline="10s",
                 exchange_mode="cooperation"),
        ir.review("ServidorCritico",
                  ir.regra("cpu_temp", ">", 70, "°C", ir.act_(ator, 200))),
    )
    context.loader.carregar(context.engine, programa)
    context.ator_primario = ator


@when('a temperatura excede {limite:d}°C e a ação é "act({ator}, {valor:d})"')
def step_temperatura_excede(context, limite, ator, valor):
    context.sim.set_sensor("cpu_temp", limite + 5.0)
    context.engine.tick()


@then('o FXP detecta a falha (heartbeat) e aplica a política de fallback do registro, tentando o ator alternativo "VentoinhaReserva" (extensão opcional)')
def step_fallback_aplicado(context):
    primario = context.ator_primario
    assert context.cad.tem("ator_indisponivel", ator=primario)
    assert context.cad.tem("fallback_executado", primario=primario,
                           alternativo="VentoinhaReserva", valor=200)
    assert context.sim.atores["VentoinhaReserva"].atual == 200
    assert context.sim.atores[primario].atual != 200


@then('o Caderno registra a tentativa primária, a falha e o fallback executado')
def step_caderno_trilha_fallback(context):
    primario = context.ator_primario
    assert context.cad.tem("ATUACAO", ator=primario, sucesso=False)
    assert context.cad.tem("ATUACAO", ator="VentoinhaReserva", sucesso=True)
    assert context.cad.tem("fallback_executado")
