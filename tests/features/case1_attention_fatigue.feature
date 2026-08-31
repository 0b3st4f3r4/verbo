# language: pt
# Cenário BDD Caso 1 — docs/PLAN.md §1.1
Funcionalidade: Salvaguarda de Atenção Cognitiva
  Cenário: Mudança de estado devido ao esgotamento da atenção do usuário
    Dado que a forma laborativa "PensarLivre" está ativa com um deadline de 3s
    Quando a leitura do sensor "attention" via FXP cai abaixo de 30% (ex: 15.0%)
    Então o runtime deve disparar uma transição "reclassify_as_equilibrium"
    E o estado da ideia deve ser gravado como ".vl" canônico no diretório de persistência
    E o Caderno registra o evento de persistência com o SHA-256 do arquivo gravado
    E após a reclassificação a forma deixa de receber ticks de manutenção
    E 0 bytes permanecem retidos em heap para a forma (verificado com contadores do runtime)
