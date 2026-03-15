#!/usr/bin/env bash
# 01_dump.sh — dcmdump showcase
# Demonstrates the various output modes of dcmdump using the ABDOM CT series.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../.."
BIN="$ROOT/target/debug/dcmdump"
FILES="$SCRIPT_DIR/../testfiles"

if [[ ! -x "$BIN" ]]; then
  echo "Binary not found — run:  cargo build --bins"
  exit 1
fi

DCM="$FILES/ABDOM_1.dcm"

echo "════════════════════════════════════════════════════════════"
echo " 1. Default dump (dataset only, truncated values)"
echo "════════════════════════════════════════════════════════════"
"$BIN" "$DCM"

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 2. Include File Meta Information header  (--meta)"
echo "════════════════════════════════════════════════════════════"
"$BIN" --meta "$DCM"

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 3. No string length limit  (--no-limit)"
echo "════════════════════════════════════════════════════════════"
"$BIN" --no-limit "$DCM" | head -30

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 4. DICOM JSON output  (--json)"
echo "════════════════════════════════════════════════════════════"
"$BIN" --json "$DCM" | head -40

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 5. DICOM XML output  (--xml)"
echo "════════════════════════════════════════════════════════════"
"$BIN" --xml "$DCM" | head -40

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 6. Dump all 5 slices at once"
echo "════════════════════════════════════════════════════════════"
for f in "$FILES"/ABDOM_*.dcm; do
  echo "--- $f ---"
  "$BIN" "$f" | grep -E "^\(0010,0010\)|^\(0008,0060\)|^\(0020,0013\)|^\(0028,0010\)|^\(0028,0011\)"
done
