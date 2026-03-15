#!/usr/bin/env bash
# 06_jp2k.sh — JPEG 2000 compression / decompression showcase
#
# Demonstrates dcmcjp2k and dcmdjp2k with the ABDOM CT test slices:
#   1. Lossless compress → decompress round-trip
#   2. Lossy compress → decompress smoke test
#   3. Batch lossless compress all test files
#   4. Batch decompress → verify metadata preserved
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../.."
CJP2K="$ROOT/target/debug/dcmcjp2k"
DJP2K="$ROOT/target/debug/dcmdjp2k"
DUMP="$ROOT/target/debug/dcmdump"
FILES="$SCRIPT_DIR/../testfiles"
TMPDIR=$(mktemp -d)

cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT

for BIN in "$CJP2K" "$DJP2K" "$DUMP"; do
  if [[ ! -x "$BIN" ]]; then
    echo "Binary not found — run:  cargo build --bins"
    exit 1
  fi
done

DCM="$FILES/ABDOM_1.dcm"

echo "════════════════════════════════════════════════════════════"
echo " 1. Lossless JPEG 2000 round-trip"
echo "════════════════════════════════════════════════════════════"
echo "  Compressing:  ABDOM_1.dcm → compressed_lossless_j2k.dcm"
"$CJP2K" -v "$DCM" "$TMPDIR/compressed_lossless_j2k.dcm"
echo ""
echo "  Decompressing: compressed_lossless_j2k.dcm → roundtrip_lossless_j2k.dcm"
"$DJP2K" -v "$TMPDIR/compressed_lossless_j2k.dcm" "$TMPDIR/roundtrip_lossless_j2k.dcm"

echo ""
echo "  Transfer syntax of each file:"
echo "    Original:     $("$DUMP" --meta "$DCM" 2>/dev/null | grep "(0002,0010)" | head -1)"
echo "    Compressed:   $("$DUMP" --meta "$TMPDIR/compressed_lossless_j2k.dcm" 2>/dev/null | grep "(0002,0010)" | head -1)"
echo "    Round-trip:   $("$DUMP" --meta "$TMPDIR/roundtrip_lossless_j2k.dcm" 2>/dev/null | grep "(0002,0010)" | head -1)"

echo ""
echo "  File sizes:"
ORIG_SIZE=$(wc -c < "$DCM" | tr -d ' ')
COMP_SIZE=$(wc -c < "$TMPDIR/compressed_lossless_j2k.dcm" | tr -d ' ')
RT_SIZE=$(wc -c < "$TMPDIR/roundtrip_lossless_j2k.dcm" | tr -d ' ')
echo "    Original:     $ORIG_SIZE bytes"
echo "    Compressed:   $COMP_SIZE bytes"
echo "    Round-trip:   $RT_SIZE bytes"

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 2. Lossy JPEG 2000 smoke test"
echo "════════════════════════════════════════════════════════════"
echo "  Compressing:  ABDOM_1.dcm → compressed_lossy_j2k.dcm"
"$CJP2K" --encode-lossy -v "$DCM" "$TMPDIR/compressed_lossy_j2k.dcm"
echo ""
echo "  Decompressing: compressed_lossy_j2k.dcm → roundtrip_lossy_j2k.dcm"
"$DJP2K" -v "$TMPDIR/compressed_lossy_j2k.dcm" "$TMPDIR/roundtrip_lossy_j2k.dcm"

echo ""
echo "  File sizes:"
LOSSY_SIZE=$(wc -c < "$TMPDIR/compressed_lossy_j2k.dcm" | tr -d ' ')
LOSSY_RT_SIZE=$(wc -c < "$TMPDIR/roundtrip_lossy_j2k.dcm" | tr -d ' ')
echo "    Original:           $ORIG_SIZE bytes"
echo "    Lossless compressed: $COMP_SIZE bytes"
echo "    Lossy compressed:   $LOSSY_SIZE bytes"
echo "    Lossy round-trip:   $LOSSY_RT_SIZE bytes"
echo "  Lossy indicators:"
echo "    Transfer Syntax:    $("$DUMP" --meta "$TMPDIR/compressed_lossy_j2k.dcm" 2>/dev/null | grep "(0002,0010)" | head -1)"
echo "    Lossy flag:         $("$DUMP" "$TMPDIR/compressed_lossy_j2k.dcm" 2>/dev/null | grep "(0028,2110)" | head -1)"

echo ""
echo "════════════════════════════════════════════════════════════"
echo " 3. Batch lossless compress all 5 ABDOM slices"
echo "════════════════════════════════════════════════════════════"
mkdir -p "$TMPDIR/batch"
for f in "$FILES"/ABDOM_*.dcm; do
  NAME=$(basename "$f")
  "$CJP2K" "$f" "$TMPDIR/batch/$NAME"
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
  "$DJP2K" "$f" "$TMPDIR/roundtrip/$NAME"
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
echo "  Showing metadata header of JPEG 2000 compressed file:"
"$DUMP" --meta "$TMPDIR/compressed_lossless_j2k.dcm" 2>/dev/null | head -30
echo "  ..."

echo ""
echo "Done — all JPEG 2000 demos complete ✓"
