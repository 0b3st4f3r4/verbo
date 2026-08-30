# PLAN-AUDIT.md — Auditoria cruzada dos documentos de planejamento

> Auditoria solicitada sobre [`docs/PLAN.md`](PLAN.md), cruzada com
> [`docs/FORMAL.md`](FORMAL.md), [`docs/MANIFESTO.md`](MANIFESTO.md),
> [`AGENTS.md`](../AGENTS.md) e [`README.md`](../README.md).
> Método: leitura integral dos cinco documentos em três eixos —
> **contradições**, **lacunas de especificação** e **mensurabilidade dos
> critérios de aceite**. Referências indicam arquivo §/linha.

---

## Veredito geral

O tripé MANIFESTO → FORMAL → PLAN é coeso e bem referenciado cruzadamente; o
cheat sheet (`VBL-CHEATSHEET.md`) foi conferido linha a linha contra a
`FORMAL.md` e está **fiel** à spec; o registro mínimo do FXP (FORMAL §6) dá
denominador objetivo às métricas de cobertura; e o §7 do PLAN é um bom exemplo
de demanda rastreada. Porém:

1. há **uma contradição direta** que produziria rejeição de programas válidos
   se a Etapa 2 seguir o PLAN à risca (A1);
2. há **divergência ontológica** entre MANIFESTO e FORMAL sobre `source_path`
   obrigatório (A2);
3. as **semânticas de `keep()`, transições e precedência de regras estão
   indefinidas** — e dois dos três cenários BDD da Etapa 1 dependem delas (B1–B4);
4. o **modelo de atribuição de energia por forma não existe** em nenhum
   documento, e toda a cadeia de métricas do AC depende dele (B6);
5. **~12 critérios de aceite não são mensuráveis como escritos** (C1–C12).

Nada disso exige código novo — são decisões de documentação — mas o grupo P1
abaixo precisa ser resolvido **antes** de escrever os primeiros BDDs, senão a
Etapa 1 cristaliza violações de spec em testes.

---

## A. Contradições diretas entre documentos

**A1. PLAN §2.1 × FORMAL §3 — atributos obrigatórios.**
PLAN (linha 82): *"A AST deve validar metadados materiais obrigatórios
(`horizon`, `source_path`, `maintenance_deadline`, `cost_bytes`)"*. A FORMAL
(linhas 46–54 e nota da linha 97) exige apenas `value` e `horizon`;
`source_path`, `maintenance_deadline` e `cost_bytes` são **opcionais**. Um EC
que implemente o PLAN literalmente rejeitará programas válidos pela EBNF.
**Correção:** reescrever o item do PLAN como *"validar os obrigatórios
(`value`, `horizon`) e a aplicabilidade dos opcionais por conjugação
(`maintenance_deadline`/`exchange_mode` só em `nonequilibrium`; `cost_bytes`
só em `equilibrium`)"*. Recomendação adicional: decidir se
`maintenance_deadline` passa a ser **obrigatório** em `nonequilibrium` na
v1.6 — sem ele, nenhuma forma laborativa colapsa jamais, violando o espírito
das Leis 3 e 4 do MANIFESTO.

**A2. MANIFESTO Lei 2 × FORMAL §3 — sensor obrigatório.**
MANIFESTO (linha 36): *"toda forma deve estar vinculada a um sensor físico
(via FXP)"*. A FORMAL define `source_path` como atributo opcional, e o
checklist do AD (AGENTS §1.1) só exige `horizon`. **Correção:** decisão do AD
— tornar `source_path` obrigatório na gramática, ou registrar no MANIFESTO a
exceção (formas sem sensor têm contabilidade por atribuição global, cf. B6).

**A3. FORMAL §4.7 × AGENTS (aceite do FXP) — falha de sensor × fallback
automático.**
A FORMAL: sensor ausente/falho ⇒ condição **não avaliada** + alerta no
Caderno; zero jamais é presumido. O AGENTS exige: *"Drivers de fallback e
simulação são ativados automaticamente quando o dispositivo real não está
acessível"*. São situações distintas (não registrado × registrado porém
inacessível), mas a fronteira não está definida: sensor registrado cuja
leitura falha — pula o tick ou injeta dado sintético? **Correção:** escrever
na FORMAL §4.7 que dado sintético só circula em modo simulado/híbrido
**explícito**, sempre marcado (`measurement_status`) no Caderno; em modo real,
falha ⇒ não avalia + alerta.

**A4. AGENTS (aceite do FXP) × FORMAL §2/§3 — "unidades validadas em tempo de
compilação".**
A gramática não possui operadores aritméticos (`expression = string | number |
identifier`, linha 75): não existe "somar Watts com Celsius" para rejeitar. E
a unidade do `threshold` só pode ser validada contra a grandeza do sensor no
**registro do FXP**, que existe em runtime (linha 93). **Correção:** reformular
o critério como (a) rejeição sintática de literal sem unidade onde a grandeza
a exige, e (b) validação em runtime contra o registro, coberta por teste no CI.

**A5. MANIFESTO §3 × FORMAL §4.1 — persistência assimétrica do `equilibrium`.**
O MANIFESTO define `equilibrium` como "estabilidade em suporte não volátil".
O diagrama da FORMAL (linhas 121–123) anota persistência em disco **somente**
na aresta NEQ→EQ; a aresta EV→EQ não persiste. **Correção:** definir que toda
forma `equilibrium` vive em disco (a materialização em memória é detalhe de
runtime), ou justificar a exceção da reclassificação a partir de `event`.

**A6. PLAN §1.2 (TDD) × FORMAL §4.7 — teste "zero ou ausente" mistura dois
comportamentos opostos.**
O item de teste diz: *"Injetar leituras de sensor com valor zero ou ausente …
assegurar que a dissolução executa a desalocação completa"*. A §4.7 exige o
**oposto** para o caso "ausente": a condição não pode disparar (zero é leitura
física válida; ausência não é zero). **Correção:** separar em dois testes —
(i) leitura `0.0` é avaliada normalmente e pode disparar regras; (ii) sensor
ausente ⇒ nenhum `when` avaliado, alerta no Caderno, nenhum disparo falso.
Escrito como está, o teste sancionaria uma violação da spec.

**A7. FORMAL §3 (nota `exchange_mode`) × PLAN Etapa 2 — referência pendular
sem item de trabalho.**
A FORMAL (linha 96) diz: *"o efeito semântico pleno será definido na Etapa 2
(cf. PLAN.md)"* — mas o PLAN Etapa 2 não menciona `exchange_mode` uma única
vez. **Correção:** incluir item explícito na Etapa 2, ou adiar formalmente o
efeito semântico e corrigir a nota da FORMAL.

---

## B. Lacunas de semântica que bloqueiam a Etapa 1

**B1. Quem emite `keep()`?** Os dois exemplos canônicos (`PensarLivre`,
`TradingEspeculativo`) e os cenários BDD 1 e 2 **não têm bloco `main`** — e
`keep()` só existe como statement de `main`. Sem emissor de `keep`,
`PensarLivre` colapsa aos 3 s por starvation de manutenção **antes** de
qualquer leitura de `attention`: o Caso 1 deixaria de testar o que anuncia.
Decidir (e escrever na FORMAL §4.1): (a) regras de revisão mantêm a forma
implicitamente enquanto nenhuma condição dispara; (b) o harness de teste
mantém; ou (c) manutenção implícita até o primeiro deadline.

**B2. Matriz de transições incompleta.** O diagrama de estados não tem aresta
`event → nonequilibrium`: `reclassify_as_nonequilibrium` sobre uma forma
`event` é legal? E sobre uma forma já dissolvida? Enumerar a matriz 3×3 de
transições legais e o comportamento fora dela (erro de runtime? no-op
registrado no Caderno?).

**B3. Reset de prazos na reclassificação.** Ao reclassificar: o `horizon`
reinicia? Qual o `maintenance_deadline` de uma forma que virou
`nonequilibrium` sem ter declarado o atributo? Qual o `cost_bytes` de uma
NEQ→EQ sem o atributo (necessário para contabilizar em `DiskBytes`)? Sem
isso, os testes de transição da Etapa 2 não têm oráculo.

**B4. Conflito e precedência de regras.** Duas regras da mesma `review`
disparam no mesmo tick — ambas executam (em ordem de declaração) ou só a
primeira? `horizon` expira no mesmo tick em que um `when` dispara — quem
vence? A `dissolve` de regra diferencia-se da expiração natural no Caderno?

**B5. Reviews duplicadas/órfãs.** A sintaxe permite `review X` para forma
inexistente e duas `review X` para a mesma forma; a semântica não define
(erro de parse? merge de regras?).

**B6. Modelo de atribuição de energia por forma — a maior lacuna técnica.**
FORMAL §4.2 manda calcular "vazamento energético (potência × duração)" **por
forma**, mas o runtime dispõe de `cpu_power` **global** (0–500 W). Como
repartir a potência entre N formas ativas (pro-rata por `cost_bytes`? por
tempo de CPU? por sensor próprio da forma)? Toda a cadeia de métricas do AC
(Joules por forma, erro ≤ 2%) depende desse modelo, inexistente nos cinco
documentos. Definir na FORMAL v1.6 e cobrir com teste determinístico no
simulador.

**B7. Semântica de `safety_limit` e da rejeição.** `Ventoinha` tem max 255 /
safety 200, e o BDD Caso 3 envia exatamente `act(Ventoinha, 200)` — o limite
é inclusivo? Comando acima do `safety_limit`: rejeita, clampeia ou executa com
alerta? Quem registra a rejeição? E `min 0` permite desligar a ventoinha com
calor alto — permitido?

**B8. Dono do fallback de ator.** FORMAL §4.3: "o *runtime* pode executar
fallback"; PLAN §3.3: atores alternativos são "configurados por política" no
FXP; BDD Caso 3: "*o FXP* deve detectar a falha e tentar o ator alternativo".
Definir: fallback é política do **registro do FXP** (primary → fallbacks); o
runtime só recebe o resultado. O Caso 3 testa então o FXP, não o runtime.

**B9. `notify_shutdown` sem objeto.** "Desligamento das cargas secundárias
associadas à forma" — não há sintaxe que associe uma carga a uma forma.
Enquanto não houver, fixar o comportamento verificável da versão: registra
evento no Caderno, não dissolve, não interrompe a `action_list` (já dito na
§4.6) — e garantir que o teste do PoC (`tokens > 2500`) valide exatamente isso.

**B10. Relógio virtual × relógio de parede.** O tick de "1 s virtual"
(FORMAL §4.2; linha 92: durações sub-segundo avaliadas na granularidade do
tick) convive com métricas de parede (transição ≤ 100 µs, leitura ≤ 1 ms).
Declarar: o runtime roda sobre relógio virtual **injetável** (CI rápido e
determinístico); métricas de latência são de parede, medidas em benches
dedicados (criterion). Sem isso, o Caso 1 custa 3 s reais por execução e os
testes térmicos não são repetíveis.

**B11. FXP: protocolo ou API?** "Mensagens assíncronas com serialização
binária" (PLAN §3.1) e latências "remotas" (AGENTS EIF) pressupõem rede, mas
não existe schema de mensagem (campos, opcode, endianness, timeout, ack) nem
definição de transporte local × remoto. O entregável da Etapa 3 deve incluir
"FXP message schema v1" antes de qualquer driver — o critério "serializa/
desserializa sem perda" só é testável com esse schema.

**B12. Orçamento de string × orçamento de memória.** Strings são ilimitadas
(FORMAL §2), mas o AGENTS exige ≤ 256 B por forma `event`. Um `value` longo
estoura o orçamento sozinho. Definir limite de string na spec, ou medir o
orçamento "exceto `value`", ou dar orçamento próprio ao `value`.

**B13. Permissões de atores.** FORMAL §6: "a segurança dos atores é
responsabilidade do FXP, que impõe limites **e permissões**" — permissões
nunca definidas: qualquer `.vl` pode agir sobre qualquer ator (inclusive cap
de CPU da máquina). Se multi-tenancy está fora de escopo na versão, dizer isso
explicitamente; senão, definir o modelo mínimo (ator com dono, política por
programa).

**B14. Persistência NEQ→EQ sem especificação.** O BDD Caso 1 exige "estado
gravado de forma persistente no disco", mas nenhum documento define formato,
caminho, ou como a forma persistida recarrega. Item de spec necessário para a
Etapa 2 e para o E2E da Etapa 4.

**B15. "Grafo de verbos" nunca é definido.** AGENTS e PLAN falam em "grafo de
formas ativas", "motor de grafo assíncrono" e O(N log N) por tick, mas a
linguagem não tem arestas entre formas (revisões referenciam sensores, não
outras formas). Definir o grafo (nós = formas; arestas = ?) ou renomear a
estrutura para "loop de tick + tabela de formas" e reancorar a métrica.

---

## C. Critérios de aceite não mensuráveis como escritos

| # | Critério (onde) | Problema | Reescrita sugerida |
|---|---|---|---|
| C1 | AD: "≥ 0,5 violações/KLOC" (AGENTS §1.1) | Meta de detecção com piso: código limpo reprova o AD; incentivo perverso | 100% de PRs revisadas + ≥ 90% de detecção em exercícios calibrados (violações injetadas) + 0 violações escapando para `main` |
| C2 | GQT: "falsos positivos/negativos: 0" | Absoluto irreal | "0 flakes não triados; todo flip de teste gera issue em ≤ 24 h" |
| C3 | AC: "erro energético ≤ 2%" × sensor `cpu_power` ±5% (FORMAL §6) | O erro do sensor domina; 2% é inalcançável | Orçamento de erro: ≤ erro do sensor (5%) + 1% do método; ou referência externa em laboratório |
| C4 | AC: "logging ≤ 0,1 W" | Atribuição causal de 0,1 W é ruído (RAPL varia ~1–3 W) | Manter overhead de CPU ≤ 1% + limite de taxa de escrita + bench A/B logger on/off |
| C5 | EIF: "atuação ≤ 50 ms até mudança física observável" | Inércia mecânica de ventoinha > 50 ms mesmo com comando aplicado | Separar "ack do driver ≤ 50 ms" de "efeito físico ≤ 500 ms (atuadores mecânicos)" |
| C6 | EC: "parser cobre 95% da especificação" | % de uma spec não tem denominador | Matriz de rastreabilidade: produção EBNF × nota semântica → id de teste; 100% das produções, ≥ 95% das notas com ≥ 1 teste |
| C7 | GQT: "100% dos cenários definidos na especificação" | A FORMAL não enumera cenários; denominador inexistente | O artefato da C6 vira o denominador canônico |
| C8 | BDD Caso 1: "nenhuma CPU adicional deve ser consumida" | Inverificável como frase | "após reclassificar: 0 ticks de manutenção e 0 bytes retidos (contador do alocador)" |
| C9 | EC: "≤ 100 µs (em hardware de referência)" | Hardware de referência nunca definido | Fixar máquina de referência (modelo de CPU anotado no PLAN) e método (criterion, p50/p95, N iterações) |
| C10 | Etapa 1 done: "cobertura de casos de erro > 80%" | Ambíguo (linhas? cenários?) | "≥ 1 teste por cláusula de erro da FORMAL" (sensor ausente, ator inexistente, valor fora de limite, forma sem `value`/`horizon`, review órfã, `keep` de forma inexistente) |
| C11 | §7: "o modelo local responde corretamente" | Sem N, sem taxa, sem oráculo | Banco fixo de 20 prompts (10 sintaxe, 10 semântica) × 3 execuções; aceito com ≥ 90% validadas por parser + rubrica GQT; resultado versionado |
| C12 | §7: "a janela de 4096 tokens impede carregar a spec" | Premissa não medida: FORMAL tem 15.261 chars ≈ 3,5–4,2 mil tokens (estimativa) | Medir com o tokenizer do Qwen; se uma versão aparada (sem mermaid/exemplos) couber, adicionar toggle "spec aparada" na UI |

Nota sobre C11: confirmado que o blueprint Python **não parseia texto**
(`prototype/verbolang-complete-blueprint.py` constrói formas
programaticamente) — hoje não existe oráculo sintático para o critério do §7.
Ele depende da Etapa 2 ou de um mini-validador dedicado; e o banco de prompts
deve ser entregue na Etapa 1.

---

## D. Higiene e pontos menores

- **D1.** A decisão Rust × C nunca é tomada (AGENTS §1.3 e PLAN §2.2 carregam
  as duas); orçamentos de memória/latência são sensíveis à linguagem. Criar
  ponto de decisão formal na saída da Etapa 1, com registro de rationale.
- **D2.** AGENTS exige suíte a cada commit; não há configuração de CI no repo
  nem escolha de runner no PLAN Etapa 1. Nomear o runner como entregável da
  Etapa 1.
- **D3.** A FORMAL não tem nenhum exemplo de `event` nem de `equilibrium` (os
  quatro exemplos são `nonequilibrium`). A matriz de conformidade (C6) precisa
  dos três; adicionar um exemplo mínimo por conjugação.
- **D4.** Explicitar a fronteira mock × simulador: na Etapa 1 é mock em
  processo, sem schema binário; o simulador físico (§6.5) evolui para módulo
  na Etapa 3.
- **D5.** `cargo-tarpaulin` está em manutenção precária; preferir
  `cargo-llvm-cov` na lista de ferramentas do AGENTS §3.
- **D6.** BDD Caso 2 diz "sensor térmico da CPU"; usar o símbolo canônico
  `cpu_temp` do registro para os testes serem rastreáveis.
- **D7.** Aliases do FXP (FORMAL §6: `attention` → `human_attention`): falta
  definir comportamento verificável — leitura por alias é idêntica? O Caderno
  registra o nome usado ou o canônico?

---

## E. Plano de correção sugerido

| Prioridade | Quando | Itens |
|---|---|---|
| **P1** | Antes dos primeiros BDDs (Etapa 1) | A1, A3, A6, B1, B2, B4 — decisões de semântica das quais os testes dependem. Uma PR única: "FORMAL v1.6 — semântica de manutenção, transições e falhas" + ajustes nos Casos 1–3 do PLAN |
| **P2** | Antes da Etapa 2 | A2, A4, A5, A7, B3, B5, B6, B7, B8, B10, B14, B15 + reescrita das métricas C1–C10 |
| **P3** | Antes da Etapa 3 | B9, B11, B12, B13, C11, C12, D1–D7 |

Esforço estimado: P1 ≈ 1 PR de documentação; P2 ≈ 2–3 PRs. Nenhum item exige
código novo ainda — e resolvê-los agora evita reescrever testes depois.

---

> **Status (pós-correção):** itens A1–A7, B1–B15, C1–C12 e D1–D7 aplicados em
> `FORMAL.md`, `PLAN.md`, `AGENTS.md`, `VBL-CHEATSHEET.md` e `MANIFESTO.md`
> (este relatório é o registro de mudanças; os documentos canônicos não
> carregam versionamento, por decisão do autor). Decisões do AD pendentes de
> ratificação: partilha energética igualitária (B6), manutenção implícita por
> revisão ativa (B1), persistência em `.vl` canônico (B14).
