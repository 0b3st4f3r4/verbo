# Exemplos executáveis

Os exemplos do repositório, prontos para rodar com o FXP simulado — todos
passam no `vbl check`. Na instalação local, os arquivos vivem em
`examples/`; aqui, ao lado deste capítulo, em `reference/exemplos/`.

| Programa | O que demonstra | Capítulo |
|---|---|---|
| [`example1_free_thinking.vl`](../../examples/example1_free_thinking.vl) | fadiga de atenção com reclassificação graciosa | [Formas](guide/formas.md) |
| [`example2_speculative_trading.vl`](../../examples/example2_speculative_trading.vl) | subversão térmica com atuação via FXP | [Reviews](guide/revisoes.md) |
| [`example5_main_task.vl`](../../examples/example5_main_task.vl) | bloco `main` — `keep()` e `act()` periódicos | [Reviews](guide/revisoes.md) |
| [`example6_sensor_ausente.vl`](../../examples/example6_sensor_ausente.vl) | sensor fora do registro: erro de compilação e a fuga `--allow-unregistered` | [FXP](guide/fxp.md) |

Cada programa é um `vbl run` de distância:

```bash
vbl run examples/example2_speculative_trading.vl \
    --ticks 8 --set cpu_temp=90 --ledger tmp-logs/demo.vcad
vbl ledger-verify tmp-logs/demo.vcad
```

> [!TIP]
> Os capítulos da trilha usam exatamente estes arquivos nos boxes
> "Experimente" — rode o comando, abra o `.vcad.jsonl` exportado e compare
> com o que o capítulo descreve. É o ciclo de estudo recomendado.
