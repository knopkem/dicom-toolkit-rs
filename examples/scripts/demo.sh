#!/usr/bin/env bash
# demo.sh — full dicom-toolkit-rs CLI demonstration
#
# Runs all showcase scripts in order against the ABDOM CT test series.
# Builds the workspace first if the binaries are not yet present.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../.."

# ─── build ────────────────────────────────────────────────────────────────────
if [[ ! -x "$ROOT/target/debug/dcmdump" ]]; then
  echo "Building workspace…"
  (cd "$ROOT" && cargo build --bins)
fi

banner() {
  echo ""
  echo "╔══════════════════════════════════════════════════════════════╗"
  printf  "║  %-61s║\n" "$1"
  echo "╚══════════════════════════════════════════════════════════════╝"
}

pause() {
  if [[ -t 0 ]]; then
    read -r -p "  Press Enter to continue…" _
  fi
}

# ─── 01: dump ─────────────────────────────────────────────────────────────────
banner "01 · dcmdump — print DICOM file contents"
bash "$SCRIPT_DIR/01_dump.sh"
pause

# ─── 02: network ──────────────────────────────────────────────────────────────
banner "02 · echoscu + storescu + storescp — network transfer"
bash "$SCRIPT_DIR/02_network.sh"
pause

# ─── 03: query ────────────────────────────────────────────────────────────────
banner "03 · findscu — C-FIND query examples"
bash "$SCRIPT_DIR/03_query.sh"
pause

# ─── 04: img2dcm ──────────────────────────────────────────────────────────────
banner "04 · img2dcm — PNG → DICOM Secondary Capture"
bash "$SCRIPT_DIR/04_img2dcm.sh"
pause

# ─── 05: jpegls ───────────────────────────────────────────────────────────────
banner "05 · dcmcjpls + dcmdjpls — JPEG-LS compress / decompress"
bash "$SCRIPT_DIR/05_jpegls.sh"

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  All demos complete ✓                                        ║"
echo "╚══════════════════════════════════════════════════════════════╝"
