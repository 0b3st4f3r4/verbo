# language: en
# Cenário BDD Caso 3 — docs/PLAN.md §1.1
Feature: Actuation Resilience
  Scenario: Primary actor fails and fallback is triggered
    Given the actor "Fan" is not responding
    When the temperature exceeds 70°C and the action is "act(Fan, 200)"
    Then the FXP detects the failure (heartbeat) and applies the registry fallback policy, trying the alternative actor "ReserveFan" (optional extension)
    And the Ledger records the primary attempt, the failure and the executed fallback
