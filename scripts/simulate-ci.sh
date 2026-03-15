#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: bash scripts/simulate-ci.sh [options]

Run the local equivalent of the commands in .github/workflows/ci.yml.

Options:
  --step <name>   Run only the specified step. May be repeated.
  --list-steps    Print the available step names and exit.
  -h, --help      Show this help text.

Available steps:
  check   cargo check --workspace --all-targets
  test    cargo test --workspace
  fmt     cargo fmt --all -- --check
  clippy  cargo clippy --workspace --all-targets -- -D warnings
  doc     cargo doc --workspace --no-deps
  deny    cargo deny check
EOF
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export RUSTFLAGS="${RUSTFLAGS:--Dwarnings}"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:--Dwarnings}"

ALL_STEPS=(check test fmt clippy doc deny)
SELECTED_STEPS=()

is_valid_step() {
  local wanted="$1"

  for step in "${ALL_STEPS[@]}"; do
    if [[ "$step" == "$wanted" ]]; then
      return 0
    fi
  done

  return 1
}

while (($#)); do
  case "$1" in
    --step)
      if (($# < 2)); then
        echo "error: --step requires a step name" >&2
        exit 1
      fi

      if ! is_valid_step "$2"; then
        echo "error: unknown step: $2" >&2
        usage
        exit 1
      fi

      SELECTED_STEPS+=("$2")
      shift
      ;;
    --list-steps)
      printf '%s\n' "${ALL_STEPS[@]}"
      exit 0
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
  shift
done

if ((${#SELECTED_STEPS[@]} == 0)); then
  SELECTED_STEPS=("${ALL_STEPS[@]}")
fi

run_step() {
  local step="$1"

  case "$step" in
    check)
      echo "==> [check] cargo check --workspace --all-targets"
      cargo check --workspace --all-targets
      ;;
    test)
      echo "==> [test] cargo test --workspace"
      cargo test --workspace
      ;;
    fmt)
      echo "==> [fmt] cargo fmt --all -- --check"
      cargo fmt --all -- --check
      ;;
    clippy)
      echo "==> [clippy] cargo clippy --workspace --all-targets -- -D warnings"
      cargo clippy --workspace --all-targets -- -D warnings
      ;;
    doc)
      echo "==> [doc] cargo doc --workspace --no-deps"
      cargo doc --workspace --no-deps
      ;;
    deny)
      echo "==> [deny] cargo deny check"
      if ! cargo deny --version >/dev/null 2>&1; then
        echo "error: cargo-deny is required for the local CI simulation." >&2
        echo "Install it with: cargo install cargo-deny --locked" >&2
        exit 1
      fi
      cargo deny check
      ;;
    *)
      echo "error: unsupported step dispatch: $step" >&2
      exit 1
      ;;
  esac
}

for step in "${SELECTED_STEPS[@]}"; do
  run_step "$step"
done

echo "Local CI simulation completed successfully."
