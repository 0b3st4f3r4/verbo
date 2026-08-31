# language: en
# Cenário BDD Caso 1 — docs/PLAN.md §1.1
Feature: Cognitive Attention Safeguard
  Scenario: State change due to the user's attention running out
    Given the laborative form "PensarLivre" is active with a deadline of 3s
    When the "attention" sensor reading via FXP drops below 30% (e.g. 15.0%)
    Then the runtime must fire a "reclassify_as_equilibrium" transition
    And the idea state must be saved as canonical ".vl" in the persistence directory
    And the Ledger records the persistence event with the SHA-256 of the written file
    And after the reclassification the form no longer receives maintenance ticks
    And 0 bytes remain retained on the heap for the form (verified with runtime counters)
