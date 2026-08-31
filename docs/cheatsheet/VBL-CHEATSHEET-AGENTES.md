# VerboLang — cheat sheet denso para agentes

Linguagem de baixo nível alinhada ao Materialismo Computacional: não existe dado inerte. Toda estrutura é uma **forma** com suporte físico, horizonte de validade e custo termodinâmico, auditados no **Caderno** (log). O mundo físico só é acessado por nomes simbólicos do **FXP** — sensores (entrada) e atores (saída); nunca caminhos `/sys/...` nem `/dev/...`.

## Ontologia

- `event`: transiente; morre por `horizon`; sem manutenção.
- `equilibrium`: persistente; sem manutenção; custo em `cost_bytes`.
- `nonequilibrium`: laborativo; manutenção contínua obrigatória; **colapsa** se `maintenance_deadline` expirar sem `keep()` nem revisão ativa (revisão ativa mantém implicitamente a cada tick).
- Toda forma exige `value` e `horizon`, nesta ordem; o parser rejeita sem eles.

## Gramática

```text
programa   = { forma | review } [ main ]
forma      = event|equilibrium|nonequilibrium Id { value: expr, horizon: duração [, atributos] }
atributos  = source_path:"sensor_fxp" | maintenance_deadline:duração
           | exchange_mode:"cooperation"|"extraction" | cost_bytes:int
           | currency:"CpuCycles"|"DiskBytes"|"PowerWatts" | classification:"texto"
             (no máximo uma vez cada; maintenance_deadline/exchange_mode só em
              nonequilibrium — o primeiro obrigatório; cost_bytes só em equilibrium)
review     = review Id { when sensor op limite -> ação {, ação} }
             op = < > <= >= == != ; limite = número | N% | W | °C
ação       = dissolve | subvert | reclassify_as_equilibrium
           | reclassify_as_nonequilibrium | notify_shutdown | act(Ator, expr)
main       = main { keep(Id) | act(Ator, expr) | every duração { ... } }
duração    = número s|ms|us|ns (decimais: 2.5s) ; expr = "string"|número|identificador
```

Erro de compilação: atributo repetido ou vírgula antes de `}`; `keep` fora do `main`; atributo na conjugação errada; acento/ç em identificador (`SensorAtencaoAlta`, nunca `SensorAtençãoAlta` — strings aceitam acentos e escapes `\" \\ \n \t`, ≤256 bytes); review de forma inexistente; uma única review por forma (no máximo uma — regras não são mescladas, a review é exclusiva). Comentários `//` e `/* */`.

## Semântica — tick de 1 s virtual

O runtime lê os sensores no FXP, avalia os `when` na ordem declarada (antes da verificação de prazos; `dissolve`/`subvert` encurtam as regras seguintes da mesma review), verifica prazos, executa as ações e registra tudo (leituras, transições, atuações, Joules) no Caderno.

- `subvert`: interrupção de prioridade máxima; substitui o `value` por `"poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"`, dissolve a forma no mesmo tick e **não cancela** as ações seguintes da regra — continuam e são executadas (um `act` é enviado ao FXP). Legítimo só: superação de limite termodinâmico (térmico/consumo) ou repetição sem propósito.
- `horizon` é absoluto (desde a criação); reclassificação não o renova. Legais: event→equilibrium, equilibrium⇄nonequilibrium, nonequilibrium→nonequilibrium (keep). **Não há retorno a event**: `reclassify_as_nonequilibrium` sobre `event` é transição ilegal — erro registrado; a forma permanece event.
- `dissolve` encerra (dissolvida); reclassify muda a conjugação. `notify_shutdown` desliga cargas secundárias associadas: **não dissolve** a forma nem interrompe as ações seguintes.
- `act(Ator, valor)`: assíncrono; o FXP valida contra limites inclusivos — fora do limite, rejeitado: o FXP não envia o comando e o fato fica registrado no Caderno (`actor_rejected_value`). Falha do ator → alerta no Caderno.
- **Falha de sensor (falha de I/O)**: a condição **não é avaliada** naquele tick e registra alerta no Caderno. Sensor ausente **nunca** vira `0.0` (ausência não é zero: `0.0` é válido, zero é uma leitura válida, leitura física válida) — tratá-lo como zero causaria disparos falsos (disparo falso, falso positivo) e condições falsas.
- Sensores canônicos: `"attention"` (%), `"cpu_temp"` (°C), `"cpu_power"` (W). Unidades `s ms us ns`, `W`, `°C`, `%` — threshold com unidade é convertido antes da comparação.

## Exemplos

```verbolang
nonequilibrium SpeculativeTrading {
    value: "lucro_arbitragem_alta_frequencia",
    horizon: 7s,
    source_path: "cpu_temp",
    maintenance_deadline: 2s,
    exchange_mode: "extraction"
}
review SpeculativeTrading {
    when cpu_temp > 85°C -> subvert, act(CpuPowerCap, 50)
}
```

```verbolang
// main roda a cada tick
nonequilibrium ImportantTask {
    value: "tarefa_critica",
    horizon: 30s,
    maintenance_deadline: 5s
}
main {
    every 4s { keep(ImportantTask) }
    every 10s { act(StatusLed, "green") }
}
```

Forma mínima com duração decimal: `event SensorLuz { value: "luz_do_solo", horizon: 2.5s }`.

Responda usando exatamente estas palavras-chave e estruturas. Todo código VerboLang vai em bloco cercado ```verbolang — nunca solto. Nunca coloque atributo fora da conjugação: `cost_bytes` só em `equilibrium`; `maintenance_deadline` e `exchange_mode` só em `nonequilibrium`. O que não estiver aqui: diga que desconhece — não invente sintaxe nem semântica.
