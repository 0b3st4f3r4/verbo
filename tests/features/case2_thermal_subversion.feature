# language: en
# Cenário BDD Caso 2 — docs/PLAN.md §1.1
Feature: Sabotage of Predatory Processing
  Scenario: Thermal overload in a speculative trading loop
    Given the task "TradingEspeculativo" is running at high frequency
    When the "cpu_temp" sensor reaches 86.5°C (limit of 85.0°C) via FXP
    Then the runtime must invoke the "subvert()" operator
    And the action "act(CpuPowerCap, 50)" must be sent to the corresponding actor via FXP
    And the trading logical value must be replaced by the canonical poetic value "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"
    And the subverted form processing must cease in the same tick (dissolution within ≤ 1 virtual tick)
