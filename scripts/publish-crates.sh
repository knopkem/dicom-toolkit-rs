#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: bash scripts/publish-crates.sh [options]

Publish dicom-toolkit-rs crates to crates.io in dependency order.

Options:
  --plan          Print the publish order and versions, then exit.
  --from <crate>  Resume publishing at the specified crate.
  --skip-check    Skip the initial `cargo check --workspace`.
  --no-wait       Do not wait for each published version to appear on crates.io.
  --allow-dirty   Pass `--allow-dirty` to `cargo publish`.
  -h, --help      Show this help text.
EOF
}

PLAN_ONLY=false
RUN_CHECK=true
WAIT_FOR_INDEX=true
ALLOW_DIRTY=false
START_AT=""

while (($#)); do
  case "$1" in
    --plan)
      PLAN_ONLY=true
      ;;
    --from)
      if (($# < 2)); then
        echo "error: --from requires a crate name" >&2
        exit 1
      fi
      START_AT="$2"
      shift
      ;;
    --skip-check)
      RUN_CHECK=false
      ;;
    --no-wait)
      WAIT_FOR_INDEX=false
      ;;
    --allow-dirty)
      ALLOW_DIRTY=true
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

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

CRATES=(
  "dicom-toolkit-core"
  "dicom-toolkit-dict"
  "dicom-toolkit-data"
  "dicom-toolkit-jpeg2000"
  "dicom-toolkit-image"
  "dicom-toolkit-net"
  "dicom-toolkit-codec"
  "dicom-toolkit-tools"
)

export CARGO_REGISTRIES_CRATES_IO_PROTOCOL="${CARGO_REGISTRIES_CRATES_IO_PROTOCOL:-sparse}"

if command -v python3 >/dev/null 2>&1; then
  PYTHON_BIN="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON_BIN="python"
else
  echo "error: python3 or python is required" >&2
  exit 1
fi

METADATA_JSON="$(cargo metadata --no-deps --format-version 1)"

crate_version() {
  local crate="$1"
  "$PYTHON_BIN" -c '
import json
import sys

crate = sys.argv[1]
metadata = json.load(sys.stdin)
for package in metadata["packages"]:
    if package["name"] == crate:
        print(package["version"])
        break
else:
    raise SystemExit(f"unknown crate: {crate}")
' "$crate" <<<"$METADATA_JSON"
}

wait_for_crates_io() {
  local crate="$1"
  local version="$2"
  local url="https://crates.io/api/v1/crates/${crate}"

  for attempt in $(seq 1 30); do
    if curl -fsSL "$url" | "$PYTHON_BIN" -c '
import json
import sys

wanted = sys.argv[1]
data = json.load(sys.stdin)
versions = {entry["num"] for entry in data.get("versions", [])}
raise SystemExit(0 if wanted in versions else 1)
' "$version"
    then
      echo "    ${crate} ${version} is visible on crates.io"
      return 0
    fi

    echo "    Waiting for ${crate} ${version} to appear on crates.io (attempt ${attempt}/30)..."
    sleep 10
  done

  echo "error: timed out waiting for ${crate} ${version} to appear on crates.io" >&2
  return 1
}

ensure_clean_worktree() {
  if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    return 0
  fi

  if [[ -n "$(git status --short)" ]]; then
    echo "error: working tree is dirty; commit or stash changes first, or rerun with --allow-dirty" >&2
    exit 1
  fi
}

start_index=0
if [[ -n "$START_AT" ]]; then
  found=false
  for i in "${!CRATES[@]}"; do
    if [[ "${CRATES[$i]}" == "$START_AT" ]]; then
      start_index="$i"
      found=true
      break
    fi
  done

  if [[ "$found" != true ]]; then
    echo "error: unknown crate for --from: $START_AT" >&2
    exit 1
  fi
fi

if [[ "$PLAN_ONLY" == true ]]; then
  echo "Publish order:"
  for crate in "${CRATES[@]}"; do
    printf '  - %s %s\n' "$crate" "$(crate_version "$crate")"
  done
  exit 0
fi

if [[ "$ALLOW_DIRTY" != true ]]; then
  ensure_clean_worktree
fi

if [[ "$RUN_CHECK" == true ]]; then
  cargo check --workspace
fi

for ((i = start_index; i < ${#CRATES[@]}; i++)); do
  crate="${CRATES[$i]}"
  version="$(crate_version "$crate")"

  cmd=(cargo publish -p "$crate")
  if [[ "$ALLOW_DIRTY" == true ]]; then
    cmd+=(--allow-dirty)
  fi

  echo "==> Publishing ${crate} ${version}"
  "${cmd[@]}"

  if [[ "$WAIT_FOR_INDEX" == true ]]; then
    wait_for_crates_io "$crate" "$version"
  fi
done
