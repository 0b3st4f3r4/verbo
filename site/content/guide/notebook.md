# O Caderno — contabilidade termodinâmica

**Neste capítulo:** o que o Caderno registra, como a energia é atribuída às
formas, o formato binário `.vcad` com cadeia SHA-256 e as três maneiras de
auditar um log — do `jq` ao verificador externo.

## O princípio

Nada acontece sem registro. Cada tick grava no Caderno: leituras de sensores,
vazamentos energéticos, transições, atuações, alertas e fins de forma — com
o timestamp do relógio virtual. O Caderno é **assíncrono**: grava em buffer
com flush periódico para nunca interferir no consumo que está medindo (a
métrica de projeto é ≤ 1% de overhead de CPU; a medição A/B está no
[relatório da Etapa 4](../../../docs/reports/STAGE-4-REPORT.md)).

## O vocabulário de eventos

Os `kind` que você vai encontrar (v1.1, normalizados para inglês; os logs
históricos v1 em português são aceitos pelos leitores):

| Kind | O que significa |
|---|---|
| `INFO` | eventos de ciclo: forma conjugada, `exchange_mode`, alívio termodinâmico |
| `LEAK` | vazamento energético: `P/N × duração` da forma naquele tick |
| `SENSOR_READ` | leitura de sensor com valor e timestamp |
| `ALERT` | condição de revisão disparada; sensor indisponível; divergências |
| `SUBVERSION` · `subvert_applied` | o operador agiu; o novo valor (poético canônico) |
| `ACTUATION` | comando a ator: valor pedido, valor aplicado, sucesso/rejeição |
| `dissolve_rule` · `dissolve_horizon` · `collapse_maintenance` · `dissolve_subvert` | os quatro fins de forma — a causa morre com a forma |
| `review_short_circuit` · `review_after_dissolution` · `actor_rejected_value` | as cláusulas de cortesia e rejeição (capítulos 4 e 5) |

## A atribuição de energia

A potência lida no tick (`cpu_power`, global) é repartida **igualmente**
entre as formas ativas: cada uma registra `P/N × duração_do_tick`, convertida
para sua `currency` — ciclos, bytes ou watts. É um orçamento honesto e
explícito: a partilha é por forma ativa, `source_path` de potência serve às
regras de revisão e **não** altera a partilha (evita dupla contagem). Metering
por forma é extensão futura; o orçamento de erro do método está em
[AGENTS §1.4](https://github.com/0b3st4f3r4/verbo#readme) (erro do sensor ±5%
+ 1% do método).

No primeiro run do [capítulo 2](installation.md): 1 forma ativa × 150 W × 1 s =
**150 J** num único `LEAK` — e o total do arquivo fecha com o acumulado.

## O formato `.vcad`

Em produção o Caderno grava num formato binário compacto: frames
`tamanho | linha | hash` — cada evento carrega o hash que o encadeia ao
anterior (cadeia SHA-256). A linha é o evento em JSON; o hash cobre o evento
**e** o hash anterior, então alterar qualquer byte do meio quebra a cadeia.
Um frame parcial no fim (flush pendente) é tolerado na leitura. O formato
completo — cabeçalho, frames, rodapé `VFIM` e verificação — está em
[NOTEBOOK-FORMAT-v1](../../../docs/NOTEBOOK-FORMAT-v1.md).

## Três maneiras de auditar

**1. A verificação forte (agente externo):**

```bash
vbl ledger-verify tmp-logs/demo.vcad
# cadeia SHA-256 ÍNTEGRA: 11 evento(s) no arquivo; atuações 1/1 ok;
# divergências (alertas): 1
# cabeça da cadeia: 8257faf7827bdce6…
```

"ÍNTEGRA" significa: recomputou a cadeia inteira do arquivo e bateu com a
cabeça — ninguém alterou, inseriu ou removeu evento. `divergências (alertas)`
não é defeito: é a contagem de eventos `ALERT` (condição disparada, sensor
ausente...) que a auditoria deve ler junto.

**2. O JSONL exportado (análise com `jq`):**

```bash
jq -r '[.tick, .kind, .detail] | @tsv' tmp-logs/demo.vcad.jsonl
```

**3. O painel ao vivo (sem tocar no arquivo):**

```bash
make web                     # http://127.0.0.1:8188/web/metrics.html
core/target/debug/vbl run examples/example5_main_task.vl \
    --ticks 5000 --ledger tmp-logs/demo.vcad
```

Escolha o ledger no seletor e veja Joules acumulados, atuações, gráfico de
energia e o feed de eventos chegando em SSE. O painel **lê** o Caderno —
verificar a cadeia continua sendo papel do `ledger-verify`.

> [!TIP]
> Os logs reais das cargas E2E do projeto estão versionados em
> `logs/stage4/` e `logs/stage5/` — com os relatórios de verificação
> externa. Um `.vcad` de verdade é o melhor material de estudo.

## Próximo passo

[Receitas e anti-padrões](recipes.md): programas completos para os
problemas clássicos — e os padrões que o revisor ontológico rejeita.
