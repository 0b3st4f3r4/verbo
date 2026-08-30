#!/usr/bin/env bash
# serve-local-llm.sh — sobe o Qwen3-4B-Instruct-2507-FP8 via vLLM
# Endpoint: http://127.0.0.1:8000/v1
# A chave é lida de LOCAL_VLLM_KEY no ambiente ou de ~/.dsh/.credentials.yaml
#
# Parte do projeto VerboLang — licenciado sob a GNU GPL-3.0
# (ver LICENSE na raiz do repositório).
set -euo pipefail

# Reduz fragmentação de VRAM (recomendado pelo PyTorch para GPUs pequenas)
export PYTORCH_CUDA_ALLOC_CONF="${PYTORCH_CUDA_ALLOC_CONF:-expandable_segments:True}"
# Desativa o sampler flashinfer (evita compilação com nvcc)
export VLLM_USE_FLASHINFER_SAMPLER="${VLLM_USE_FLASHINFER_SAMPLER:-0}"

MODEL="Qwen/Qwen3-4B-Instruct-2507-FP8"   # ~4.8 GB, cabe na sua GPU
SERVED="qwen3-4b"                          # nome que o Harness vai usar
CTX=4096                                   # tamanho do contexto

# Flags otimizadas para pouca VRAM
TUNING=(
  --gpu-memory-utilization 0.90
  --kv-cache-dtype fp8
  --enforce-eager
  --max-num-seqs 16
  --attention-backend TRITON_ATTN
  --kernel-config '{"enable_jit_warmup": false, "enable_flashinfer_autotune": false, "enable_cutedsl_warmup": false}'
  --enable-auto-tool-choice
  --tool-call-parser qwen3_coder
)

# Tenta obter a chave do ambiente ou do arquivo de credenciais
KEY="${LOCAL_VLLM_KEY:-$(awk '/^LOCAL_VLLM_KEY:/{print $2}' "${DSH_HOME:-$HOME/.dsh}/.credentials.yaml" 2>/dev/null || true)}"
if [[ -z "${KEY:-}" ]]; then
  echo "LOCAL_VLLM_KEY não encontrada (env ou ~/.dsh/.credentials.yaml)" >&2
  exit 1
fi

echo ">>> Subindo o modelo $MODEL como '$SERVED' na porta 8000"

# ── UI de consulta (chat.html) ──────────────────────────────────
# Vigia em subshell: quando o /v1/models responde 200, sobe um servidor
# estático (python3 http.server) na RAIZ do repositório — a página precisa
# alcançar ../docs/VBL-CHEATSHEET.md para o modo "+ VerboLang" — e abre o
# navegador. A chave vai no fragmento da URL (#…), que o navegador não envia
# ao servidor estático. Desative com LOCAL_LLM_UI=0; porta com
# LOCAL_LLM_UI_PORT (8188).
if [[ "${LOCAL_LLM_UI:-1}" != "0" ]]; then
  UI_PORT="${LOCAL_LLM_UI_PORT:-8188}"
  UI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  UI_URL="http://127.0.0.1:${UI_PORT}/scripts/chat.html#k=${KEY}&u=http%3A%2F%2F127.0.0.1%3A8000%2Fv1&m=${SERVED}&c=${CTX}"
  (
    http_pid=""
    for _ in $(seq 1 120); do                    # ~10 min de paciência
      sleep 5
      # vLLM (processo pai, pós-exec) morreu → encerrar o que vigorou
      if ! kill -0 "$PPID" 2>/dev/null; then
        [ -n "$http_pid" ] && kill "$http_pid" 2>/dev/null
        exit 0
      fi
      if curl -fsS -H "Authorization: Bearer ${KEY}" http://127.0.0.1:8000/v1/models >/dev/null 2>&1; then
        if ! curl -fsS "http://127.0.0.1:${UI_PORT}/scripts/chat.html" >/dev/null 2>&1; then
          python3 -m http.server "$UI_PORT" --bind 127.0.0.1 --directory "$UI_DIR" >/dev/null 2>&1 &
          http_pid=$!
          sleep 1
        fi
        if curl -fsS "http://127.0.0.1:${UI_PORT}/scripts/chat.html" >/dev/null 2>&1; then
          command -v xdg-open >/dev/null 2>&1 && xdg-open "$UI_URL" >/dev/null 2>&1
          echo ">>> UI de consulta: ${UI_URL%%\#*}  (modelo ${SERVED}, ctx ${CTX})"
          break                                   # UI no ar → fase de guarda
        fi
        echo ">>> aviso: não consegui servir a UI na porta ${UI_PORT}" >&2
        break
      fi
    done
    # Fase de guarda: servidor estático vive enquanto o vLLM viver
    while kill -0 "$PPID" 2>/dev/null; do sleep 10; done
    [ -n "$http_pid" ] && kill "$http_pid" 2>/dev/null
    exit 0
  ) &
fi

exec vllm serve "$MODEL" \
  --served-model-name "$SERVED" \
  --host 127.0.0.1 \
  --port 8000 \
  --max-model-len "$CTX" \
  --api-key "$KEY" \
  "${TUNING[@]}"
