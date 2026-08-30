# SETUP-LOCAL-LLM.md — Modelo local via vLLM para o ecossistema VerboLang

> Hardware de referência: **Acer Nitro V 15 (ANV15-41)** — Ryzen 7 7735HS (8C/16T, 4.83 GHz),
> RTX 4050 Mobile **6 GB GDDR6** (Ada, FP8/INT4), Radeon 680M iGPU, **30 GB DDR5** (+23 GB swap),
> NVMe 476 GB (~306 GB livres), Ubuntu 26.04 (kernel 7.0), driver NVIDIA open 595.84,
> vLLM 0.28.0 + PyTorch (pyenv 3.13.7), Docker 29.1.3.

> **Decisão vigente:** o pipeline mantém apenas **dois modelos** — o orquestrador
> **GLM-5.3/Flash (cloud, via DSH)** e o único modelo local, **Qwen3-4B-Instruct-2507**
> (FP8), servido por `scripts/serve-local-llm.sh`. Os demais candidatos avaliados em
> ago/2026 foram descartados (ver §1); os detalhes da pesquisa completa constam do
> registro da sessão de trabalho de ago/2026.
>
> **Uso primário do modelo local:** testes de comunicação entre LLMs via VerboLang
> (PoC `verbolang-llm-poc.py` — dois agentes conversando sob o runtime, com revisões,
> Caderno e cadeia SHA-256). O serviço de embeddings (§3.3) é infraestrutura opcional
> do detector de loops, não um terceiro modelo de trabalho.

---

## 0. Pré-requisito crítico: nós de dispositivo NVIDIA

O driver carrega (`lsmod` OK, NVRM 595.84) mas **`/dev/nvidia*` não existem** e o
`nvidia-modprobe` não está instalado — sem isso, `nvidia-smi` falha (exit 9) e o vLLM
não vê a GPU. Correção (uma vez, com sudo):

```bash
sudo apt install nvidia-modprobe
sudo nvidia-modprobe -u -c=0     # cria /dev/nvidia0, /dev/nvidiactl, /dev/nvidia-uvm
nvidia-smi                       # deve listar a RTX 4050 com 6144 MiB
```

Se persistir após upgrade de kernel: `sudo apt install --reinstall nvidia-driver-595-open` e reinicie.

---

## 1. Cenário 2026 → decisão de 2 modelos (pesquisa ago/2026, fontes no final)

A pesquisa de ago/2026 avaliou os candidatos abaixo para esta máquina (6 GB VRAM /
30 GB RAM). A decisão final foi **não** manter um segundo modelo local:

| Modelo | Veredito |
|---|---|
| **Qwen3-4B-Instruct-2507** | ✅ **Mantido** — 4B denso, FP8 ~4,8 GB, 100% em VRAM (~30 tok/s medidos); único modelo local |
| Qwen3-Coder-30B-A3B-Instruct | ❌ Descartado — exigia offload GPU+CPU (UD-Q3/Q4, ~12–18 GB, 17 GB de download) com ~10–20 tok/s estimados; a qualidade extra não compensou a complexidade operacional |
| Mellum2 (JetBrains, 12B MoE) | ❌ Descartado — papel coberto pelo par GLM cloud + Qwen3-4B |
| GPT-OSS-20B | ❌ Descartado — ~13 GB MXFP4, exige offload |
| Devstral Small (24B) | ❌ Descartado — denso, Q4 ~13 GB, offload lento |
| GLM-4.5-Air (106B-A12B) | ❌ ~60 GB mesmo em Q4 |
| GLM-5 (745B) | ❌ inviável local — é o orquestrador cloud |
| Kimi K2.6 / DeepSeek V3.2 / Llama 4 | ❌ classe servidor |

---

## 2. Recomendação estratégica — pipeline de 2 camadas

| Camada | Modelo | Papel no VerboLang | Custo |
|---|---|---|---|
| **Orquestrador** | GLM-5.3/flash (cloud, via DSH) | Revisão AD (ontologia), arquitetura, decisões de especificação, refatorações grandes, contexto longo | por token |
| **Local único** | Qwen3-4B-2507 FP8 (vLLM, GPU-only) | **Testes de comunicação inter-LLM via VerboLang** (PoC `verbolang-llm-poc.py`: cada agente é um ator no FXP, sensores medem o diálogo, `subvert` derruba loops) e bulk offline: rascunhos Gherkin, casos de erro da EBNF, stubs Rust/C e FXP — privado (nada sai da máquina), custo zero por token | energia |

Divisão prática: o GLM cloud decide e revisa; o Qwen3-4B executa o volume local.
Quando a qualidade do 4B não bastar para uma tarefa pesada (ex.: o parser completo
da Etapa 2), a tarefa escala para o GLM — critério do AD, sem modelo local intermediário.

### Por que não um GLM local
GLM-5 é 745B e GLM-4.5-Air 106B — classe servidor. O GLM já atua como orquestrador
cloud; localmente, o Qwen3-4B cobre o bulk com o hardware que existe.

---

## 3. Instalação

### 3.0 Lições de VRAM em 6 GB (validadas na prática, ago/2026)

O Qwen3-4B-FP8 só subiu após 5 iterações — cada uma com causa distinta:

1. **Desktop GNOME reserva ~0,4 GB** — o vLLM enxerga 5,64 GiB, não 6,0
2. **CUDA graphs + torch.compile estouram o orçamento** → `--enforce-eager` (custo ~10–15% em decode, libera ~0,6 GB)
3. **KV cache em FP8** (`--kv-cache-dtype fp8`) — metade da memória; Ada tem FP8 nativo
4. **Sampler do flashinfer exige nvcc** (JIT em C++!) → `VLLM_USE_FLASHINFER_SAMPLER=0` usa o caminho nativo PyTorch
5. **`--attention-backend TRITON_ATTN`** + `--kernel-config '{"enable_jit_warmup": false, ...}'` — evita os demais caminhos que exigem toolkit CUDA
6. `PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True` + `--max-num-seqs 16` + ctx 4096

Todos já embutidos em `scripts/serve-local-llm.sh`.

### 3.1 Via vLLM

```bash
# Perfil validado: Qwen3-4B-Instruct-2507-FP8, sobe em ~90 s (pesos em cache)
bash scripts/serve-local-llm.sh    # http://127.0.0.1:8000/v1
```

Resultado do teste de aceitação:
- `GET /v1/models` → `qwen3-4b` (root: Qwen3-4B-Instruct-2507-FP8) ✓
- `POST /v1/chat/completions` → 37 tokens em ~2,0 s (~30 tok/s) ✓
- KV cache: 9.344 tokens, concorrência 2,28x @ 4096 ctx ✓

Medições no uso real (PoC inter-LLM, ago/2026):
- Diálogo de 23 turnos: 5.656 tokens, latência 2,7–4,9 s por chamada (~55–70 tok/s ponta a ponta)
- Os quatro fins de conversa validados contra o LLM real: **Alívio** (`horizon`), **notify_shutdown** (orçamento de tokens), **subvert** (repetição sem propósito) e **colapso** (manutenção expirada)
- Caderno: cadeia SHA-256 íntegra em todas as execuções

> Nota honesta: para **um usuário** com offload pesado, llama.cpp com GGUFs
> quantizados (`--n-gpu-layers` + `-ct q8_0`) costuma dar throughput melhor e é mais
> simples de tunar; vLLM brilha com concorrência (vários subagentes batendo no
> endpoint). Teste os dois; o endpoint OpenAI-compatível é igual para o DSH.
> Se um dia instalar o toolkit CUDA (`sudo apt install nvidia-cuda-toolkit`),
> os mitigações 4/5 tornam-se desnecessários e o flashinfer volta a ser útil.

### 3.2 Alternativa GPU-only (rápida, sem offload)

```bash
vllm serve Qwen/Qwen3-4B-Instruct-2507-FP8 \
  --max-model-len 4096 --gpu-memory-utilization 0.90 --kv-cache-dtype fp8 \
  --enforce-eager --attention-backend TRITON_ATTN --max-num-seqs 16 --port 8001
```

### 3.3 Nó de embeddings (opcional) — detector de loop do PoC inter-LLM

O `verbolang-llm-poc.py` mede `dialogo_loop_risk` por embeddings. **Sem** nó de
embeddings, cai automaticamente para hashing de n-gramas (100% stdlib, sem VRAM)
— suficiente para os testes cotidianos. Para similaridade semântica real:

```bash
vllm serve Qwen/Qwen3-Embedding-0.6B --task embed --port 8002
VBL_EMBED_URL=http://127.0.0.1:8002/v1 python3 prototype/verbolang-llm-poc.py
```

- **Conflito de VRAM:** os 6 GB não comportam o chat e o embedding simultaneamente
  (o chat reserva 90% da VRAM). Rode o nó de embeddings com o chat desligado ou
  em outra máquina.
- O método ativo (HTTP × hashing) fica **registrado no Caderno** — cada
  `loop_risk` carrega a procedência da medição.
- Para o segundo nó em cloud (GLM-5.3/Flash) — **validado dos dois lados, ago/2026**:
  assinantes do Coding Plan devem usar a rota `https://api.z.ai/api/coding/paas/v4`;
  a rota pay-as-you-go (`/api/paas/v4`) devolve HTTP 429 "Insufficient balance".
  Atalho de topologia: `VBL_AGENTE_URL=<rota> VBL_AGENTE_MODEL=glm-5.3-flash
  VBL_AGENTE_KEY_ENV=ZAI_API_KEY`. O GLM-5.3-Flash é *reasoning*: com
  `VBL_MAX_TOKENS=512`, 1 de 7 turnos veio vazio (o reasoning consumiu o
  orçamento) — use 768+ ou aceite o alerta do Caderno.

---

## 4. Integração com o DeepSeek Harness

Em `~/.dsh/settings.yaml`, seção `llm-pi-ai.providers` (mesma estrutura do `zai`):

```yaml
llm-pi-ai:
  providers:
    local:
      baseUrl: http://127.0.0.1:8000/v1
      apiKeyEnv: LOCAL_VLLM_KEY        # adicionar a chave em ~/.dsh/.credentials.yaml
      models:
        - id: qwen3-4b
          name: Qwen3-4B-2507 (local)
```

Com isso, subagentes/workflows do DSH podem receber `provider: local` e o VerboLang
ganha um worker offline dentro do mesmo pipeline de agentes
(AD/GLM cloud → decisão e revisão; local → execução bulk).

---

## 5. Fontes (pesquisa via LangSearch, ago/2026)

- Qwen3-4B-Instruct-2507-FP8 (HF, modelo servido localmente): https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507-FP8
- Qwen3-Embedding-0.6B (HF, nó opcional de embeddings do PoC): https://huggingface.co/Qwen/Qwen3-Embedding-0.6B
- GLM-5 (745B, MIT, fev/2026 — orquestrador cloud): https://localaimaster.com/models/glm-5 · https://effloow.com/articles/glm-5-open-source-frontier-model-setup-guide-2026
- GLM-4.5-FP8 (HF, MIT): https://huggingface.co/zai-org/GLM-4.5-FP8
- Melhores LLMs locais por tier de VRAM (ago/2026): https://benchlm.ai/best/local-llm
- LLMs locais por tarefa em 4/6/8 GB: https://www.mayhemcode.com/2026/06/best-local-llms-for-4gb-6gb-and-8gb.html

> Os candidatos descartados (Qwen3-Coder-30B-A3B, Mellum2, GPT-OSS-20B,
> Devstral Small e a análise de KV-cache do 30B) e suas fontes de pesquisa foram
> removidos nesta revisão do documento; a pesquisa completa de ago/2026 permanece
> no registro da sessão de trabalho.

> Números de tok/s são estimativas da comunidade para a classe de hardware — valide
> com o seu workload antes de fechar a arquitetura (princípio da honestidade
> termodinâmica: meça, não presuma).
