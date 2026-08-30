# VBL-CHEATSHEET.md — VerboLang em uma página (injetável como prompt de sistema)

> Artefato canônico da demanda do [`PLAN.md`](PLAN.md) §7 (caminho "a" — modelo
> local com conhecimento da linguagem). Mantido junto da spec: qualquer mudança
> relevante na [`FORMAL.md`](FORMAL.md) deve refletir aqui. A UI de consulta
> local ([`scripts/chat-local.html`](../scripts/chat-local.html)) injeta este
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
  `maintenance_deadline` expirar sem `keep()`.
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
  | 'maintenance_deadline' ':' duration  (* apenas nonequilibrium *)
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

Comentários: `//` de linha e `/* ... */` de bloco. Strings: `"..."` com escapes
`\"`, `\\`, `\n`, `\t`. Identificadores: `[a-zA-Z_][a-zA-Z0-9_]*`.

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
- **`dissolve`**: encerra a forma. **`reclassify_as_equilibrium` /
  `reclassify_as_nonequilibrium`**: mudam a conjugação (formas persistem ou
  passam a exigir `keep()`).
- **`notify_shutdown`**: desliga cargas secundárias associadas; não dissolve a
  forma nem interrompe as ações seguintes.
- **`act(Ator, valor)`**: comando assíncrono via FXP; o FXP valida o valor
  contra limites (mínimo, máximo, segurança) e registra comando, valor,
  timestamp e custo no Caderno. Falha do ator → alerta no Caderno e possível
  fallback do runtime.
- **Falha de sensor**: a condição **não é avaliada** naquele tick — um sensor
  ausente nunca é tratado como leitura `0.0` (zero é leitura física válida).
- **Nomes simbólicos**: `source_path` e atores são símbolos do FXP
  (ex.: `"cpu_temp"`, `CpuPowerCap`, `Ventoinha`) — **nunca** caminhos de
  sistema (`/sys/...`, `/dev/...`); o mapeamento físico é do registro do FXP.
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

Ao responder sobre VerboLang, use **exatamente** estas palavras-chave e esta
estrutura. Se algo não estiver coberto aqui, diga que desconhece em vez de
inventar sintaxe ou semântica.
