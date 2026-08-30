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
exec vllm serve "$MODEL" \
  --served-model-name "$SERVED" \
  --host 127.0.0.1 \
  --port 8000 \
  --max-model-len "$CTX" \
  --api-key "$KEY" \
  "${TUNING[@]}"
