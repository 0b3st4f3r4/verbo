# ==============================================================================
# fxp_sim — suporte de teste da Etapa 1 (docs/PLAN.md §1.3)
# ==============================================================================
# Componentes:
#   blueprint  — carregador do protótipo de referência (SUT da Etapa 1)
#   support    — consulta ao Caderno para asserções
#   simulator  — simulador físico determinístico do FXP (PLAN §6.5, esqueleto;
#                evolui para módulo do FXP real na Etapa 3)
#   mocks      — fronteira mock em processo (sem schema binário)
#   ir         — builders do IR (formas/reviews/main) usado pelos testes
#   loader     — carrega IR no engine (mock do front-end; o parser real é da
#                Etapa 2 — PLAN §2.1)
#   contract   — validador estrutural do IR: cláusulas de erro da FORMAL §3
#
# Convenção: nenhum teste usa aleatoriedade — o relógio virtual é avançado
# pelo runtime injetado (FORMAL §4.2) e as séries de sensores são roteirizadas.
