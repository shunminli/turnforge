#!/usr/bin/env bash
# Foreground only: the calling terminal owns the server; Ctrl-C stops it.
set -euo pipefail

turnforge_data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/turnforge"
turnforge_ollama_bin="${TURNFORGE_OLLAMA_BIN:-$turnforge_data_dir/ollama/0.33.3/ollama}"
if [[ ! -x "$turnforge_ollama_bin" ]]; then
  echo "Ollama 0.33.3 not found. See docs/local-llm.md or set TURNFORGE_OLLAMA_BIN." >&2
  exit 1
fi

# No credential/proxy inheritance is needed for this local test service.
unset OPENAI_API_KEY TURNFORGE_API_KEY ANTHROPIC_API_KEY OLLAMA_API_KEY
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
export NO_PROXY='*' no_proxy='*'
export OLLAMA_HOST='127.0.0.1:11434'
export OLLAMA_NO_CLOUD=1
export OLLAMA_MODELS="$turnforge_data_dir/models"
export OLLAMA_CONTEXT_LENGTH=8192
export OLLAMA_NUM_PARALLEL=1
export OLLAMA_MAX_LOADED_MODELS=1
export OLLAMA_KEEP_ALIVE=5m

exec "$turnforge_ollama_bin" serve
