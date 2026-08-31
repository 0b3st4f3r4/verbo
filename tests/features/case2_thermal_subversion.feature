# language: pt
# Cenário BDD Caso 2 — docs/PLAN.md §1.1
Funcionalidade: Sabotagem de Processamento Predatório
  Cenário: Sobrecarga térmica em loop de trading especulativo
    Dado que a tarefa "TradingEspeculativo" está rodando em alta frequência
    Quando o sensor "cpu_temp" atinge 86.5°C (limite de 85.0°C) via FXP
    Então o runtime deve invocar o operador "subvert()"
    E a ação "act(CpuPowerCap, 50)" deve ser enviada ao ator correspondente via FXP
    E o valor lógico de trading deve ser substituído pelo valor poético canônico "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"
    E o processamento da forma subvertida deve cessar no mesmo tick (dissolução em ≤ 1 tick virtual)
