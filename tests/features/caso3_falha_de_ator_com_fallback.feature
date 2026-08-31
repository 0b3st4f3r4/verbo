# language: pt
# Cenário BDD Caso 3 — docs/PLAN.md §1.1
Funcionalidade: Resiliência de Atuação
  Cenário: Ator principal falha e fallback é acionado
    Dado que o ator "Ventoinha" não está respondendo
    Quando a temperatura excede 70°C e a ação é "act(Ventoinha, 200)"
    Então o FXP detecta a falha (heartbeat) e aplica a política de fallback do registro, tentando o ator alternativo "VentoinhaReserva" (extensão opcional)
    E o Caderno registra a tentativa primária, a falha e o fallback executado
