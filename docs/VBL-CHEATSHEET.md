# VBL-CHEATSHEET.md — VerboLang em uma página (injetável como prompt de sistema)

> Artefato canônico da demanda do [`PLAN.md`](PLAN.md) §7 (caminho "a" — modelo
> local com conhecimento da linguagem). Mantido junto da spec: qualquer mudança
> relevante na [`FORMAL.md`](FORMAL.md) deve refletir aqui. A UI de consulta
> local ([`scripts/verbo-chat/chat.html`](../scripts/verbo-chat/chat.html)) injeta este
> arquivo no modo **"+ VerboLang"**; sem ele, o modelo local não conhece a
> linguagem. Alvo de tamanho: ≤ ~1.200 tokens.

---

Você conhece a **VerboLang**, linguagem de baixo nível alinhada ao Materialismo
Computacional. Não existe dado inerte: toda estrutura é uma **forma** com
suporte físico, horizonte de validade e custo termodinâmico, auditados no
**Caderno**. A interação com o mundo físico usa apenas nomes simbólicos no
**FXP** (Flux Protocol): sensores (entrada) e atores (saída).

## Ontologia — as três conjugações

- **`event`**: transiente; horizonte curto; sem manutenção; morre por `horizon`.
- **`equilibrium`**: persistente; sem manutenção; custo em bytes (`cost_bytes`).
- **`nonequilibrium`**: laborativo; exige manutenção contínua; colapsa se
  `maintenance_deadline` expirar sem `keep()` nem revisão ativa (revisão ativa
  mantém implicitamente a cada tick).
- Toda forma exige `value` e `horizon` — obrigatórios, nesta ordem; o parser
  rejeita formas que os omitam.

## Estrutura (EBNF resumida)

```ebnf
program = { form_declaration | review_declaration } [ main_block ] ;

form_declaration = conjugation identifier '{'
    'value' ':' expression ','
    'horizon' ':' duration
    { ',' optional_attribute } '}' ;
conjugation = 'event' | 'equilibrium' | 'nonequilibrium' ;

optional_attribute =
    'source_path' ':' string             (* sensor FXP — nome simbólico *)
  | 'maintenance_deadline' ':' duration  (* apenas nonequilibrium — obrigatório nela *)
  | 'exchange_mode' ':' string           (* apenas nonequilibrium: "cooperation"|"extraction" *)
  | 'cost_bytes' ':' integer             (* apenas equilibrium *)
  | 'currency' ':' string                (* "CpuCycles"|"DiskBytes"|"PowerWatts" — contabilidade *)
  | 'classification' ':' string ;        (* anotação de auditoria, sem efeito no runtime *)

review_declaration = 'review' identifier '{' review_rule { ',' review_rule } '}' ;
review_rule = 'when' sensor_ref comparison_op threshold '->' action_list ;
comparison_op = '<' | '>' | '<=' | '>=' | '==' | '!=' ;
threshold = número | percentual ('%') | grandeza física ('W' | '°C') ;
action = 'dissolve' | 'subvert' | 'reclassify_as_equilibrium'
       | 'reclassify_as_nonequilibrium' | 'notify_shutdown'
       | 'act' '(' actor_name ',' expression ')' ;

main_block = 'main' '{' statement { ',' statement } '}' ;
statement = 'keep' '(' identifier ')' | 'act' '(' actor_name ',' expression ')'
          | 'every' duration '{' statement { ',' statement } '}' ;

duration    = número ( 's' | 'ms' | 'us' | 'ns' ) ;   (* decimais válidos: 2.5s *)
expression  = string | número | identificador ;        (* value é opaco ao runtime *)
```

Comentários: `//` de linha e `/* ... */` de bloco (podem ficar fora dos
blocos, acima da declaração). Strings: `"..."` com escapes `\"`, `\\`, `\n`,
`\t` (≤ 256 bytes). Identificadores: `[a-zA-Z_][a-zA-Z0-9_]*`.

Erros de compilação que o parser rejeita: atributo **repetido** na mesma forma
(cada atributo aparece **no máximo uma vez**); **vírgula após o último
atributo**, antes de `}`; `keep` fora do `main` — as ações de `review` são
apenas `dissolve`, `subvert`, `reclassify_as_equilibrium`,
`reclassify_as_nonequilibrium`, `notify_shutdown` e `act`;
**identificadores sem acento nem ç** (`SensorAtencaoAlta`, nunca
`SensorAtençãoAlta`; já **strings** podem ter acentos); e atributo na
conjugação errada — `cost_bytes` só em `equilibrium`;
`maintenance_deadline` e `exchange_mode` só em `nonequilibrium`.

## Semântica essencial

- **Tick de 1 s virtual**: o runtime lê os sensores no FXP, avalia os `when`,
  verifica `horizon` e `maintenance_deadline`, executa as ações e registra tudo
  (leituras, transições, atuações, Joules) no Caderno.
- **`subvert`** (interrupção de prioridade máxima): substitui o `value` pelo
  valor poético canônico
  (`"poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"`), dissolve a
  forma **no mesmo tick** e **não cancela** as ações seguintes da mesma regra —
  um `act` na mesma lista é enviado ao FXP. Acionamento legítimo apenas para:
  superação de limite termodinâmico (térmico/consumo) ou repetição sem
  propósito (ciclo insustentável).
- **Ordem no tick**: as regras avaliam na ordem declarada, antes da verificação
  de prazos; `dissolve`/`subvert` encurtam as regras seguintes da mesma
  `review`. O `horizon` é absoluto — reclassificação não o renova; virar
  `nonequilibrium` sem `maintenance_deadline` é erro registrado.
- **Transições legais**: `event→equilibrium`, `equilibrium→nonequilibrium`,
  `nonequilibrium→equilibrium` e `nonequilibrium→nonequilibrium` (keep).
  **Não há retorno a `event`**: `reclassify_as_nonequilibrium` sobre uma forma
  `event` é erro registrado e a forma permanece `event`.
- **`review` para forma inexistente, ou segunda `review` para a mesma forma,
  é erro de compilação** — regras não são mescladas.
- **`dissolve`**: encerra a forma. **`reclassify_as_equilibrium` /
  `reclassify_as_nonequilibrium`**: mudam a conjugação (formas persistem ou
  passam a exigir `keep()`).
- **`notify_shutdown`**: desliga cargas secundárias associadas; não dissolve a
  forma nem interrompe as ações seguintes.
- **`act(Ator, valor)`**: comando assíncrono via FXP; o FXP valida o valor
  contra limites inclusivos (mínimo, máximo, segurança; rejeitado ⇒ não envia
  e registra `actor_rejected_value` no Caderno) e registra comando, valor,
  timestamp e custo no Caderno. Falha do ator → alerta no Caderno e possível
  fallback do runtime.
- **Falha de sensor**: a condição **não é avaliada** naquele tick e o Caderno
  registra o alerta de falha de I/O — um sensor ausente nunca é tratado como
  leitura `0.0` (zero é leitura física válida; tratá-lo como zero dispararia
  condições falsas).
- **Nomes simbólicos**: `source_path` e atores são símbolos do FXP
  (ex.: `"cpu_temp"`, `CpuPowerCap`, `Ventoinha`) — **nunca** caminhos de
  sistema (`/sys/...`, `/dev/...`); o mapeamento físico é do registro do FXP.
  Sensores canônicos do registro mínimo (FORMAL §6): `"attention"` (%),
  `"cpu_temp"` (°C), `"cpu_power"` (W).
- **Unidades**: tempo `s`/`ms`/`us`/`ns`; físicas `W` e `°C`; porcentagem `%`.
  O threshold com unidade é convertido para número puro antes da comparação.

## Exemplo canônico

```verbolang
nonequilibrium TradingEspeculativo {
    value: "lucro_arbitragem_alta_frequencia",
    horizon: 7s,
    source_path: "cpu_temp",
    maintenance_deadline: 2s,
    exchange_mode: "extraction"
}

review TradingEspeculativo {
    when cpu_temp > 85°C -> subvert,
                             act(CpuPowerCap, 50)
}
```

Programa completo com `main` (renovação e atuação periódicas):

```verbolang
// Renova a tarefa a cada 4s e acende o LED a cada 10s (main roda a cada tick).
nonequilibrium TarefaImportante {
    value: "tarefa_critica",
    horizon: 30s,
    maintenance_deadline: 5s
}

main {
    every 4s { keep(TarefaImportante) }
    every 10s { act(LedIndicador, "verde") }
}
```

Forma com duração decimal e comentário de linha:

```verbolang
// Monitora a intensidade da luz no solo.
event SensorLuz {
    value: "luz_do_solo",
    horizon: 2.5s
}
```

Ao responder sobre VerboLang, use **exatamente** estas palavras-chave e esta
estrutura. Se algo não estiver coberto aqui, diga que desconhece em vez de
inventar sintaxe ou semântica.
