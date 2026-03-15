#!/usr/bin/env bash
# 05_jpegls.sh — JPEG-LS compression / decompression showcase
#
# Demonstrates dcmcjpls and dcmdjpls with the ABDOM CT test slices:
#   1. Lossless compress → decompress round-trip
#   2. Near-lossless compress → decompress round-trip
#   3. Batch compress all test files
#   4. Verify round-trip integrity by comparing dcmdump output
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../.."
CJPLS="$ROOT/target/debug/dcmcjpls"
DJPLS="$ROOT/target/debug/dcmdjpls"
DUMP="$ROOT/target/debug/dcmdump"
FILES="$SCRIPT_DIR/../testfiles"
TMPDIR=$(mktemp -d)

cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT

for BIN in "$CJPLS" "$DJPLS" "$DUMP"; do
  if [[ ! -x "$BIN" ]]; then
    echo "Binary not found — run:  cargo build --bins"
    exit 1
  fi
done

DCM="$FILES/ABDOM_1.dcm"

echo "════════════════════════════════════════════════════════════"
echo " 1. Lossless JPEG-LS round-trip"
echo "════════════════════════════════════════════════════════════"
echo "  Compressing:  ABDOM_1.dcm → compressed_lossless.dcm"
"$CJPLS" -v "$DCM" "$TMPDIR/compressed_lossless.dcm"
echo ""
echo "  Decompressing: compressed_lossless.dcm → roundtrip_lossless.dcm"
"$DJPLS" -v "$TMPDIR/compressed_lossless.dcm" "$TMPDIR/roundtrip_lossless.dcm"

echo ""
echo "  Transfer syntax of each file:"
echo "    Original:     $("$DUMP" --meta "$DCM" 2>/dev/null | grep "(0002,0010)" | head -1)"
echo "    Compressed:   $("$DUMP" --meta "$TMPDIR/compressed_lossless.dcm" 2>/dev/null | grep "(0002,0010)" | head -1)"
echo "    Round-trip:   $("$DUMP" --meta "$TMPDIR/roundtrip_lossless.dcm" 2>/dev/null | grep "(0002,0010)" | head -1)"

echo ""
echo "  File sizes:"
ORIG_SIZE=$(wc -c < "$DCM" | tr -d ' ')
COMP_SIZE=$(wc -c < "$TMPDIR/compressed_lossless.dcm" | tr -d ' ')
RT_SIZE=$(wc -c < "$TMPDIR/roundtrip_lossless.dcm" | tr -d ' ')
echo "    Original:     $ORIG_SIZE bytes"
echo "    Compressed:   $COMP_SIZE bytes"
echo "    Round-trip:   $RT_SIZE bytes"

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 2. Near-lossless JPEG-LS (max deviation = 3)"
echo "════════════════════════════════════════════════════════════"
echo "  Compressing:  ABDOM_1.dcm → compressed_lossy.dcm"
"$CJPLS" -v -n 3 "$DCM" "$TMPDIR/compressed_lossy.dcm"
echo ""
echo "  Decompressing: compressed_lossy.dcm → roundtrip_lossy.dcm"
"$DJPLS" -v "$TMPDIR/compressed_lossy.dcm" "$TMPDIR/roundtrip_lossy.dcm"
echo ""

LOSSY_SIZE=$(wc -c < "$TMPDIR/compressed_lossy.dcm" | tr -d ' ')
echo "  File sizes:"
echo "    Original:           $ORIG_SIZE bytes"
echo "    Lossy compressed:   $LOSSY_SIZE bytes"
echo "    Lossless compressed: $COMP_SIZE bytes"

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 3. Batch lossless compress all 5 ABDOM slices"
echo "════════════════════════════════════════════════════════════"
mkdir -p "$TMPDIR/batch"
for f in "$FILES"/ABDOM_*.dcm; do
  NAME=$(basename "$f")
  "$CJPLS" "$f" "$TMPDIR/batch/$NAME"
  OSIZE=$(wc -c < "$f" | tr -d ' ')
  CSIZE=$(wc -c < "$TMPDIR/batch/$NAME" | tr -d ' ')
  RATIO=$(echo "scale=1; $OSIZE / $CSIZE" | bc 2>/dev/null || echo "?")
  echo "  $NAME:  $OSIZE → $CSIZE bytes  (${RATIO}:1)"
done

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 4. Batch decompress → verify metadata preserved"
echo "════════════════════════════════════════════════════════════"
mkdir -p "$TMPDIR/roundtrip"
for f in "$TMPDIR"/batch/ABDOM_*.dcm; do
  NAME=$(basename "$f")
  "$DJPLS" "$f" "$TMPDIR/roundtrip/$NAME"
  # Compare patient name from original and round-trip
  ORIG_PATIENT=$("$DUMP" "$FILES/$NAME" 2>/dev/null | grep "(0010,0010)" | head -1 || echo "")
  RT_PATIENT=$("$DUMP" "$TMPDIR/roundtrip/$NAME" 2>/dev/null | grep "(0010,0010)" | head -1 || echo "")
  if [[ "$ORIG_PATIENT" == "$RT_PATIENT" ]]; then
    echo "  $NAME:  metadata preserved ✓"
  else
    echo "  $NAME:  WARNING — metadata differs"
    echo "    orig: $ORIG_PATIENT"
    echo "    rt:   $RT_PATIENT"
  fi
done

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 5. Dump compressed file structure"
echo "════════════════════════════════════════════════════════════"
echo "  Showing metadata header of JPEG-LS compressed file:"
"$DUMP" --meta "$TMPDIR/compressed_lossless.dcm" 2>/dev/null | head -30
echo "  ..."

echo ""
echo "Done — all JPEG-LS demos complete ✓"
