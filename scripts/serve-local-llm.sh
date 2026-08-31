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

# ── UI/dashboard (INDEPENDENTE do modelo) ─────────────────────────
# Sobe a ponte webui (scripts/webui.py: estático na raiz do repo + métricas
# SSE do Caderno) ANTES do LLM e ANTES da checagem de chave: carregar a UI
# nunca depende do modelo — dashboard e métricas funcionam com ele
# carregando, no ar, sem chave ou desligado, e o badge mostra o estado real
# (inclusive 401 de navegador sem chave; cf. web/badge.js). setsid isola a
# UI do Ctrl+C do vLLM: ela vive até `make stop` (pkill webui.py). A chave
# vai no fragmento da URL (#…), que o navegador não envia ao servidor.
# Desative com LOCAL_LLM_UI=0; porta com LOCAL_LLM_UI_PORT (8188).
if [[ "${LOCAL_LLM_UI:-1}" != "0" ]]; then
  UI_PORT="${LOCAL_LLM_UI_PORT:-8188}"
  UI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  UI_URL="http://127.0.0.1:${UI_PORT}/web/#k=${KEY:-}&u=http%3A%2F%2F127.0.0.1%3A8000%2Fv1&m=${SERVED}&c=${CTX}"
  if ! curl -fsS "http://127.0.0.1:${UI_PORT}/web/" >/dev/null 2>&1; then
    if command -v setsid >/dev/null 2>&1; then
      setsid python3 "$UI_DIR/scripts/webui.py" "$UI_PORT" --root "$UI_DIR" >/dev/null 2>&1 &
    else
      python3 "$UI_DIR/scripts/webui.py" "$UI_PORT" --root "$UI_DIR" >/dev/null 2>&1 &
    fi
    sleep 1
  fi
  command -v xdg-open >/dev/null 2>&1 && xdg-open "$UI_URL" >/dev/null 2>&1
  echo ">>> UI/dashboard: ${UI_URL%%\#*}  (independente do modelo; chat usa #k=…)"
fi

# Tenta obter a chave do ambiente ou do arquivo de credenciais
KEY="${LOCAL_VLLM_KEY:-$(awk '/^LOCAL_VLLM_KEY:/{print $2}' "${DSH_HOME:-$HOME/.dsh}/.credentials.yaml" 2>/dev/null || true)}"
if [[ -z "${KEY:-}" ]]; then
  echo "LOCAL_VLLM_KEY não encontrada (env ou ~/.dsh/.credentials.yaml) — a UI/métricas seguem no ar; sem chave o chat não autentica" >&2
  exit 1
fi

echo ">>> Subindo o modelo $MODEL como '$SERVED' na porta 8000"
exec vllm serve "$MODEL" \
  --served-model-name "$SERVED" \
  --host 127.0.0.1 \
  --port 8000 \
  --max-model-len "$CTX" \
  --api-key "$KEY" \
  "${TUNING[@]}"
