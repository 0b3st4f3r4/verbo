#

<p align="center"><img src="docs/verbolog-banner.svg" alt="VerboLang" width="640"></p>

**Uma linguagem de programação onde nenhum dado é inerte.**

A VerboLang é uma linguagem de baixo nível alinhada ao **Materialismo Computacional**:
toda estrutura lógica é uma **forma** com suporte físico concreto, horizonte de validade,
custo energético e consequências termodinâmicas explícitas. A integridade do sistema não
se mede em selos de conformidade, mas em **Joules, Celsius e ciclos de CPU** — registrados
de forma auditável no **Caderno**.

```mermaid
flowchart TB
    subgraph MUNDO["Mundo físico"]
        direction LR
        S["Sensores<br/>(entrada)"]
        A["Atores<br/>(saída)"]
    end

    FXP["FXP — Flux Protocol<br/>barramento único de I/O<br/>nomes simbólicos → endpoints"]
    RT["Runtime VerboLang<br/>tick de 1 s virtual<br/>formas · revisões · keep()"]
    CAD[("Caderno<br/>log termodinâmico<br/>cadeia SHA-256")]

    S -->|"leituras"| FXP
    FXP -->|"valores auditados"| RT
    RT -->|"act(ator, valor)"| FXP
    FXP -->|"comandos"| A
    RT -->|"Joules · transições · atuações"| CAD
```

## As três conjugações da matéria

| Conjugação | Significado | Ciclo de vida |
|---|---|---|
| `event` | Transiente — o acontecer puro | Horizonte curto, expira sem manutenção |
| `equilibrium` | Sustentado — estabilidade em suporte não volátil | Persiste, mas ocupa bytes (e não é eterno) |
| `nonequilibrium` | Laborativo — esforço contra a entropia | Exige `keep()` contínuo; colapsa sem manutenção |

## Exemplo

Subversão poética por sobrecarga térmica, com atuação física via FXP:

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
                             act(CpuPowerCap, 50)   // FXP limita a potência da CPU
}
```

Quando a CPU ultrapassa 85 °C, o runtime substitui o valor da forma pelo valor poético
canônico, dissolve-a no mesmo tick e envia o comando de limitação de potência ao ator
`CpuPowerCap`. Tudo — leitura, subversão, atuação, Joules dissipados — vai para o Caderno.

## Protótipo de referência

O blueprint em Python emula engine, FXP e Caderno, e serve como especificação
executável para a portabilidade futura para Rust ou C:

```bash
python3 prototype/verbolang-complete-blueprint.py
```

- Requisitos: Python **3.10+** (testado em 3.13), apenas biblioteca padrão.
- Saída: coreografia de 12 segundos virtuais no console + `caderno_log.jsonl`
  (artefato gerado, ignorado pelo git) com a cadeia SHA-256 verificável
  (ver `Caderno.verify_chain`).

### PoC: comunicação entre LLMs via VerboLang

O [`prototype/verbolang-llm-poc.py`](prototype/verbolang-llm-poc.py) coloca **dois agentes LLM**
(Proponente ↔ Crítico) para conversar sob o runtime VerboLang — sem alterar a
spec: cada agente é um **ator** no FXP (um `POST /chat/completions`), o estado
da conversa são **sensores** numéricos (turnos, tokens, latência, risco de
loop) e a conversa é uma forma `nonequilibrium` com regras de revisão:

- `when dialogo_loop_risk > 0.85 -> subvert` — repetição sem propósito (§4.5)
- `when dialogo_tokens > 2500 -> notify_shutdown` — orçamento estourado
- `horizon 8s` — Alívio Termodinâmico: a conversa se dissolve ao esgotar

```mermaid
sequenceDiagram
    autonumber
    participant F as FXP-LLM (Runtime)
    participant P as Proponente (LLM)
    participant C as Crítico (LLM)
    participant Cad as Caderno

    loop rodada de diálogo — um turno por tick, alternando
        F->>P: act(Proponente, mensagem) — POST /chat/completions
        P-->>F: proposta
        F->>Cad: tokens, latência, Joules estimados, hash
        F->>C: act(Crítico, proposta)
        C-->>F: risco material + condição de aceite
        F->>Cad: auditoria do turno
    end
    Note over F: revisões por tick: loop_risk acima do limiar → subvert<br/>tokens acima do orçamento → notify_shutdown
    Note over F: horizon esgotado → Alívio Termodinâmico
```

```bash
bash scripts/serve-local-llm.sh        # terminal 1 — sobe o qwen3-4b local
python3 prototype/verbolang-llm-poc.py # terminal 2 — roda o diálogo auditado
```

Topologia "local primeiro": ambos os agentes usam o vLLM local. Para mover um
agente a outro nó (ex.: GLM cloud), defina `base_url`, `model` e `api_key_env`
em `AGENTES[nome]` — cada agente pode apontar para um endpoint diferente.
Atalho para "cloud dos dois lados": `VBL_AGENTE_URL` + `VBL_AGENTE_MODEL` +
`VBL_AGENTE_KEY_ENV` reconfigura todos os agentes de uma vez (assinantes do
Coding Plan devem usar a rota `https://api.z.ai/api/coding/paas/v4`; com
modelos *reasoning*, suba `VBL_MAX_TOKENS`, ex.: 512). Topologia mista
(validada): `VBL_CRITICO_URL` / `VBL_CRITICO_MODEL` / `VBL_CRITICO_KEY_ENV`
movem só o Crítico para o cloud, mantendo o Proponente no local.

Cada ramo da semântica tem um cenário de teste: `VBL_HORIZON` (fim por Alívio),
`VBL_TEMPERATURE` baixa + `VBL_LOOP_LIMITE` (`subvert` por repetição),
`VBL_SEM_KEEP=1` (colapso por manutenção expirada) e `VBL_TICKS` (duração).

O detector de repetição (`dialogo_loop_risk`) mede similaridade por
**embeddings**: se um nó expuser `/v1/embeddings` (OpenAI-compatível), basta
apontar `VBL_EMBED_URL`/`VBL_EMBED_MODEL` para ele — sem nó de embeddings, o
PoC cai automaticamente para *hashing* de n-gramas (100% stdlib, sem VRAM
extra). O método ativo fica registrado no Caderno. Nota: os 6 GB de VRAM não
comportam o chat e o modelo de embeddings simultaneamente — rode o nó de
embeddings com o chat desligado ou em outra máquina.

## Estrutura do repositório

| Arquivo | Papel |
|---|---|
| [`AGENTS.md`](AGENTS.md) | Equipe de agentes (AD, EIF, EC, AC, GQT), métricas e critérios de aceite — na raiz por convenção de harness |
| [`docs/MANIFESTO.md`](docs/MANIFESTO.md) | As seis leis do Materialismo Computacional |
| [`docs/FORMAL.md`](docs/FORMAL.md) | Especificação formal: tokens, EBNF, semântica operacional, registro FXP |
| [`docs/PLAN.md`](docs/PLAN.md) | Roadmap de execução em 5 etapas + análise de riscos |
| [`docs/SETUP-LOCAL-LLM.md`](docs/SETUP-LOCAL-LLM.md) | Pipeline de LLMs: GLM-5.3/Flash (cloud) + Qwen3-4B-2507 (local) |
| [`prototype/verbolang-complete-blueprint.py`](prototype/verbolang-complete-blueprint.py) | Protótipo de referência (FXP, runtime, Caderno, bloco `main`) |
| [`prototype/verbolang-llm-poc.py`](prototype/verbolang-llm-poc.py) | PoC: comunicação inter-LLM (agentes LLM como atores/sensores no FXP) |
| [`scripts/serve-local-llm.sh`](scripts/serve-local-llm.sh) | Sobe o modelo local Qwen3-4B-Instruct-2507-FP8 via vLLM (e abre a UI de consulta) |
| [`scripts/verbo-chat/chat.html`](scripts/verbo-chat/chat.html) | UI de consulta: chat single-file, streaming SSE, medidor de contexto, alternância puro ↔ +VerboLang |
| [`scripts/verbo-chat/verbolog.svg`](scripts/verbo-chat/verbolog.svg) | Marca do projeto (logo e favicon da UI de consulta) |
| [`docs/VBL-CHEATSHEET.md`](docs/VBL-CHEATSHEET.md) | VerboLang em uma página — cheat sheet canônico injetável como prompt de sistema |
| [`LICENSE`](LICENSE) | Licença GPL-3.0 (copyleft) |

> **Ordem de leitura sugerida:** [`docs/MANIFESTO.md`](docs/MANIFESTO.md) →
> [`docs/FORMAL.md`](docs/FORMAL.md) → [`docs/PLAN.md`](docs/PLAN.md) →
> [`AGENTS.md`](AGENTS.md).

## Roadmap

Estado atual: **especificação + protótipo de referência concluídos** (pré-Etapa 1).

1. **Etapa 1** — Suíte BDD/TDD/E2E com mocks e simulador FXP
2. **Etapa 2** — Núcleo da linguagem: lexer, parser, AST e motor de tick assíncrono
3. **Etapa 3** — FXP real: registro de dispositivos, sensores e atores
4. **Etapa 4** — Caderno de produção e validação end-to-end
5. **Etapa 5** — Qualidade, profiling termodinâmico e otimização

Detalhes e critérios de aceite por etapa em [`docs/PLAN.md`](docs/PLAN.md) e [`AGENTS.md`](AGENTS.md).

## Desenvolvimento assistido por LLMs

O projeto usa um pipeline de **dois modelos**: o orquestrador **GLM-5.3/Flash**
(cloud, revisão de ontologia e arquitetura) e o **Qwen3-4B-Instruct-2507**
(local, via vLLM — trabalho bulk offline, dados de sensores não saem da máquina).
Configuração completa em [`docs/SETUP-LOCAL-LLM.md`](docs/SETUP-LOCAL-LLM.md).

> **Atenção:** o modelo local **não conhece a VerboLang** — a especificação não
> está no seu treino e a janela de 4096 tokens impede carregá-la inteira.
> Tarefas da linguagem exigem injetar contexto no prompt: use o modo
> **"+ VerboLang"** da UI de consulta (injeta o
> [`docs/VBL-CHEATSHEET.md`](docs/VBL-CHEATSHEET.md)) ou o cheat sheet no seu
> próprio prompt. Demanda e caminhos (cheat sheet, RAG, fine-tune) em
> [`docs/PLAN.md` §7](docs/PLAN.md).

Para consulta direta ao modelo local, `bash scripts/serve-local-llm.sh` abre
automaticamente a **UI de chat** ([`scripts/verbo-chat/chat.html`](scripts/verbo-chat/chat.html))
quando o modelo termina de carregar — streaming, medidor de contexto e a
alternância puro ↔ +VerboLang.

## Licença

Distribuído sob a [GNU GPL-3.0](LICENSE) — Copyright (C) 2026 Silvano Neto.
Copyleft: cópias e derivados devem permanecer sob a mesma licença, com o
código-fonte disponível. No vocabulário do projeto: **o comum exige `keep()`** —
a abertura não é dada, é sustentada.

> Nota: a GPL-3.0 cobre o código e a documentação deste repositório. Os modelos
> de LLM citados no pipeline seguem as próprias licenças (Qwen3-4B: Apache-2.0;
> GLM: MIT) e apenas são executados localmente via API — nada de seus pesos ou
> códigos é redistribuído ou incorporado aqui.

> *Pois o Ser é movimento dialético, contínuo e termodinâmico do Real. E o nosso código,
> enfim, aprendeu a dançar com a matéria — lendo seus sinais e respondendo com atos.*
