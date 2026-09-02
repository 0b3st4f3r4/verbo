# Reviews — quando a matéria reage

**Neste capítulo:** a gramática das regras de revisão, a ordem exata de
avaliação dentro de um tick, as seis ações possíveis e a semântica fina de
`subvert` — a interrupção de prioridade máxima.

## A forma de uma regra

```ebnf
review_rule = 'when' sensor_ref comparison_op threshold '->' action_list ;
```

```verbolang
review ServidorCritico {
    when cpu_temp > 70°C  -> act(Fan, 200),
    when cpu_temp > 90°C  -> subvert, act(CpuPowerCap, 50),
    when cpu_power >= 400W -> notify_shutdown
}
```

Peças:

- **`sensor_ref`** — o nome do sensor no FXP (identificador, ex.: `cpu_temp`).
- **comparação** — `<`, `>`, `<=`, `>=`, `==`, `!=`.
- **threshold** — número puro, **porcentagem** (`30%`) ou **grandeza física**
  (`85°C`, `400W`). O valor com unidade é convertido para número antes da
  comparação, e a **grandeza é validada** contra o registro do FXP: comparar
  `cpu_temp > 400W` não compila para verdade — é uso indevido de unidade.
- **ações** — uma lista, executada na ordem declarada.

Uma forma tem **no máximo uma** `review` — regras não são mescladas; segunda
`review` para a mesma forma (ou `review` para forma inexistente) é **erro de
compilação**.

## A ordem exata do tick

A cada tick (1 s virtual), para cada forma ativa:

1. **leituras** — o runtime consulta o FXP pelos sensores referenciados;
2. **vazamento** — a potência lida é repartida igualmente entre as formas
   ativas: cada uma registra `P/N × duração` no Caderno;
3. **revisões** — as regras são avaliadas **na ordem declarada**, *antes* da
   verificação de prazos;
4. **prazos** — `maintenance_deadline` e `horizon` só agem se a forma
   **continuar ativa** ao final do passo.

Duas cláusulas de cortesia do runtime:

- **`review_short_circuit`**: se uma ação dissolve ou subverte a forma, as
  regras *seguintes da mesma review* não são avalia­das naquele tick — sem
  revogar atuações já despachadas;
- **`review_after_dissolution`**: ação sobre forma já dissolvida no mesmo
  tick é ignorada, com registro.

## As seis ações

| Ação | Efeito |
|---|---|
| `dissolve` | encerra a forma no tick (Alívio Termodinâmico imediato) |
| `subvert` | interrupção de prioridade máxima — ver abaixo |
| `reclassify_as_equilibrium` | passa a persistir em suporte não volátil (custo em bytes) e deixa de exigir manutenção |
| `reclassify_as_nonequilibrium` | passa a exigir manutenção; **erro registrado** se a forma não declarou `maintenance_deadline` |
| `notify_shutdown` | sinaliza desligamento das cargas secundárias associadas no FXP; **não** dissolve a forma e não interrompe as ações seguintes |
| `act(Ator, valor)` | envia comando ao ator via FXP — assíncrono, validado contra limites (capítulo 5) |

## `subvert`, em detalhe

`subvert` é o gesto mais radical da linguagem — e o mais restrito:

1. **Substitui o `value`** da forma pelo valor poético canônico
   `"poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"` e registra
   o evento;
2. **encerra o ciclo da forma**: dissolução **no mesmo tick** em que a
   condição disparou, com liberação imediata de recursos — e encerra também a
   avaliação das regras seguintes da mesma `review`
   (`review_short_circuit`);
3. **não cancela as demais ações da mesma regra**: a lista continua na ordem
   declarada — em particular, qualquer `act` associado é enviado. O exemplo
   canônico depende disso:

```verbolang
review SpeculativeTrading {
    when cpu_temp > 85°C -> subvert,
                             act(CpuPowerCap, 50)
}
```

A forma morre **e** a CPU é limitada — a consequência física não morre junto
com o pensamento que a causou.

> [!IMPORTANT]
> **Condições legítimas de acionamento** (MANIFESTO §5, critério de revisão
> do AD): superação de limites termodinâmicos (térmicos, de consumo) ou
> ciclos insustentáveis (repetição sem propósito). `subvert` como goto
> fashionável — "reiniciar quando der erro" — é rejeitado em revisão.

## O bloco `main`

O `main` é imperativo e mínimo — três comandos, sem variáveis:

```verbolang
main {
    every 4s  { keep(ImportantTask) },
    every 10s { act(StatusLed, "green") }
}
```

- **`keep(Forma)`** — renovação manual da manutenção (só aqui!);
- **`act(Ator, valor)`** — atuação direta, fora de revisão;
- **`every duração { ... }`** — periodicidade; os comandos rodam no tick em
  que o intervalo vence.

> [!TIP]
> Rode o exemplo 5 e observe o Caderno:
> `vbl run examples/example5_main_task.vl --ticks 30 --ledger tmp-logs/main.vcad`
> — os `keep` aparecem como eventos de manutenção, os `act` como atuações
> com custo.

## Próximo passo

[FXP — sensores e atores](fxp.md): como o `cpu_temp` da regra vira uma
leitura real — e o `act` vira um comando aceito, rejeitado ou com fallback.
