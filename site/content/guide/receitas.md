# Receitas e anti-padrões

**Neste capítulo:** programas completos para os problemas clássicos
(reação térmica, fadiga de atenção, vigilância periódica, registro
persistente, diálogo entre LLMs) e os anti-padrões que não passam pela
revisão ontológica do AD.

## Receita 1 — reação térmica com atuação

O clássico: um serviço crítico que exige ventoinha quando esquenta e cede
quando esquenta demais. Dois limiares, duas respostas proporcionais:

```verbolang
nonequilibrium ServidorCritico {
    value: "processamento_continuo",
    horizon: 3600s,
    source_path: "cpu_temp",
    maintenance_deadline: 10s,
    exchange_mode: "cooperation"
}

review ServidorCritico {
    when cpu_temp > 70°C -> act(Fan, 200),
    when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50)
}
```

A ordem das regras **é** política térmica. No formato acima, um pico a 90 °C
aciona a ventoinha **e** subverte: a regra benigna avalia primeiro, despacha
o `act(Fan, 200)`, e a grave então subverte com o limite de potência — as
ações se acumulam. Na ordem inversa, `subvert` no primeiro limiar encerraria
a avaliação (`review_short_circuit`) e a ventoinha nunca seria acionada.
Quando as ações devem se acumular, ordene do benigno ao grave; quando são
mutuamente exclusivas, do grave ao benigno. O runtime não adivinha a sua
política — o Caderno registra qual regra disparou.

## Receita 2 — fadiga de atenção com degradação graciosa

Trabalho laborativo que **não morre** quando o operador olha para outro
lugar: degrada para persistência e pode ser retomado
([`examples/example1_free_thinking.vl`](../../../examples/example1_free_thinking.vl)):

```verbolang
nonequilibrium FreeThinking {
    value: "consciencia_antineoliberal_ativa",
    horizon: 60s,
    source_path: "attention",
    maintenance_deadline: 3s,
    exchange_mode: "cooperation"
}

review FreeThinking {
    when attention < 30% -> reclassify_as_equilibrium
}
```

`reclassify_as_equilibrium` grava a forma em disco (com SHA-256 no Caderno),
zera a exigência de manutenção e **mantém as regras ativas** — atenção
voltando acima de 30% numa futura execução permite reclassificar de volta.

## Receita 3 — vigilância periódica com `main`

Renovação e atuação em cadências diferentes, sem regra de revisão:

```verbolang
nonequilibrium ImportantTask {
    value: "dados_sensiveis",
    horizon: 30s,
    source_path: "cpu_power",
    maintenance_deadline: 5s,
    exchange_mode: "cooperation"
}

main {
    every 4s  { keep(ImportantTask) },
    every 10s { act(StatusLed, "green") }
}
```

O `keep` a cada 4 s renova um prazo de 5 s com folga de 1 tick — e sem ele a
forma colapsaria no primeiro vencimento. Proporção recomendada: **período do
`every` ≤ 60–80% do `maintenance_deadline`**, para tolerar perdas ocasionais
de tick.

## Receita 4 — registro persistente auditável

```verbolang
equilibrium Laudo {
    value: "resultado_da_analise_termica",
    horizon: 86400s,
    cost_bytes: 4096,
    currency: "DiskBytes",
    classification: "LaudoTecnico"
}
```

Vive em suporte não volátil, custa 4 KiB declarados (ou o tamanho real,
se omitido), morre em 24 h — nada de "para sempre". Persistida e recarregada
na próxima execução automaticamente, enquanto o horizonte não vencer.

## Receita 5 — diálogo auditado entre LLMs

O PoC [`prototype/verbolang-llm-poc.py`](https://github.com/0b3st4f3r4/verbo/blob/main/prototype/verbolang-llm-poc.py)
coloca dois agentes LLM para conversar **sob o runtime**: cada agente é um
*ator* no FXP, o estado da conversa são *sensores* numéricos (turnos, tokens,
risco de loop por similaridade de embeddings) e a conversa é uma forma
`nonequilibrium` com três saídas honestas:

```verbolang
review Dialogo {
    when dialogo_loop_risk > 0.85 -> subvert,        // repetição sem propósito
    when dialogo_tokens > 2500     -> notify_shutdown, // orçamento estourado
}
// horizon: 8s — Alívio Termodinâmico ao esgotar
```

Nenhum agente decide sozinho quando parar — a física do diálogo decide.

## Anti-padrões (o AD rejeita na revisão)

| Anti-padrão | Por que não | Em vez disso |
|---|---|---|
| `subvert` como tratamento de erro genérico | condições legítimas: limite termodinâmico ou ciclo insustentável (MANIFESTO §5) | `dissolve` + regra específica; `notify_shutdown` para cargas |
| Tratar sensor ausente como `0.0` | zero é leitura válida — gera condição falsa | deixar a condição não avaliada + ALERT (é o comportamento da linguagem) |
| `every 1s { keep(X) }` com `maintenance_deadline: 5s` | funciona até o primeiro soluço; sem folga | período ≤ 60–80% do prazo (Receita 3) |
| `horizon` gigante "para não morrer" | existência sem prazo é mentira física — e a forma persistida volta assombrando execuções futuras | horizontes honestos + reclassificação para `equilibrium` |
| Acento/ç em identificadores | o lexer rejeita (`SensorAtenção` não compila) | `SensorAtencao`; acentos só dentro de strings |
| Segunda `review` para a mesma forma | regras não são mescladas — é erro de compilação | uma `review` com múltiplos `when` |
| Ato fora dos limites (`act(Fan, 300)`) | rejeitado sem envio (`actor_rejected_value`) | respeitar `min`/`max`/`safety_limit` do registro |

## Onde ir agora

- A [especificação formal](../../../docs/FORMAL.md) — a palavra final sobre
  qualquer semântica deste livro;
- o [cheat sheet completo](../../../docs/cheatsheet/VBL-CHEATSHEET.md) — a
  linguagem em uma página;
- os [exemplos executáveis](../exemplos.md) — todos rodando com o FXP
  simulado;
- o [processo de releases](../../../docs/RELEASES.md) — como a linguagem
  nasce em versões (`v2027.0.0-alpha.0` é a primeira da linha `v2027.0`).

> [!NOTE]
> Encontrou uma divergência entre este livro e a FORMAL? A FORMAL vence —
> e a divergência é um bug de documentação: abra uma issue no
> [repositório](https://github.com/0b3st4f3r4/verbo/issues).
