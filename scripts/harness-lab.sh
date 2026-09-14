#!/usr/bin/env bash
# Use the repository's locked Rust CLI; never install a runtime or download models.
set -euo pipefail

turnforge_repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$turnforge_repo_root"

if command -v cargo >/dev/null 2>&1; then
  turnforge_cargo_bin="$(command -v cargo)"
elif [[ -x "$HOME/.cargo/bin/cargo" ]]; then
  turnforge_cargo_bin="$HOME/.cargo/bin/cargo"
else
  echo "Cargo not found. Install the repository's pinned Rust toolchain first (see README.md)." >&2
  exit 1
fi

exec "$turnforge_cargo_bin" run --quiet --locked -- lab "$@"
