#!/usr/bin/env bash
# 04_img2dcm.sh — img2dcm showcase
#
# Creates a small test PNG with Python (no extra deps required),
# then wraps it in a DICOM Secondary Capture file and dumps the result.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../.."
IMG2DCM="$ROOT/target/debug/img2dcm"
DCMDUMP="$ROOT/target/debug/dcmdump"

if [[ ! -x "$IMG2DCM" ]]; then
  echo "Binary not found — run:  cargo build --bins"
  exit 1
fi

WORK_DIR="$(mktemp -d)"
cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

# ─────────────────────────────────────────────
echo "════════════════════════════════════════════════════════════"
echo " Step 1 — Create a test PNG with Python"
echo "════════════════════════════════════════════════════════════"

# Build a 128×128 RGB gradient PNG using Python stdlib only (struct + zlib)
python3 - "$WORK_DIR/test.png" << 'PYEOF'
import sys, struct, zlib

def write_png(path, width, height):
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    rows = []
    for y in range(height):
        row = bytearray([0])          # filter byte
        for x in range(width):
            r = int(x / width  * 255)
            g = int(y / height * 255)
            b = 128
            row += bytearray([r, g, b])
        rows.append(bytes(row))

    raw = zlib.compress(b"".join(rows))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", raw))
        f.write(chunk(b"IEND", b""))
    print(f"  Written {width}x{height} RGB PNG → {path}")

write_png(sys.argv[1], 128, 128)
PYEOF

# ─────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════════════════════════"
echo " Step 2 — Wrap PNG in a DICOM Secondary Capture file"
echo "════════════════════════════════════════════════════════════"

OUT_DCM="$WORK_DIR/test.dcm"
"$IMG2DCM" \
  -p "Demo^Patient" \
  -P "DEMO_001" \
  -s "Test Study" \
  -S "Secondary Capture" \
  -v \
  "$WORK_DIR/test.png" "$OUT_DCM"

# ─────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════════════════════════"
echo " Step 3 — Dump the resulting DICOM file"
echo "════════════════════════════════════════════════════════════"
"$DCMDUMP" --meta "$OUT_DCM"

# ─────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════════════════════════"
echo " Step 4 — Export as DICOM JSON"
echo "════════════════════════════════════════════════════════════"
"$DCMDUMP" --json "$OUT_DCM" | head -50

echo ""
echo "════════════════════════════════════════════════════════════"
echo " Done — Secondary Capture DICOM written to: $OUT_DCM"
echo " (file will be cleaned up with temp dir on exit)"
echo "════════════════════════════════════════════════════════════"
