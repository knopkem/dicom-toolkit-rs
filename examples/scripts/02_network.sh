#!/usr/bin/env bash
# 02_network.sh — storescu / storescp / echoscu showcase
#
# Starts a Storage SCP on localhost:11112, sends all 5 ABDOM slices,
# then verifies the received files look correct.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../.."
DCMDUMP="$ROOT/target/debug/dcmdump"
ECHOSCU="$ROOT/target/debug/echoscu"
STORESCU="$ROOT/target/debug/storescu"
STORESCP="$ROOT/target/debug/storescp"
FILES="$SCRIPT_DIR/../testfiles"

for bin in "$DCMDUMP" "$ECHOSCU" "$STORESCU" "$STORESCP"; do
  if [[ ! -x "$bin" ]]; then
    echo "Binary not found: $bin — run:  cargo build --bins"
    exit 1
  fi
done

PORT=11112
RECV_DIR="$(mktemp -d)"
SCP_PID=""

cleanup() {
  if [[ -n "$SCP_PID" ]]; then
    kill "$SCP_PID" 2>/dev/null || true
    wait "$SCP_PID" 2>/dev/null || true
  fi
  rm -rf "$RECV_DIR"
}
trap cleanup EXIT

# ─────────────────────────────────────────────
echo "════════════════════════════════════════"
echo " Step 1 — Start Storage SCP on :$PORT"
echo "════════════════════════════════════════"
"$STORESCP" -v -a STORESCP -d "$RECV_DIR" "$PORT" &
SCP_PID=$!
sleep 0.5

if ! kill -0 "$SCP_PID" 2>/dev/null; then
  echo "ERROR: storescp failed to start (port $PORT in use?)"
  exit 1
fi
echo "storescp PID $SCP_PID  →  saving to $RECV_DIR"

# ─────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════"
echo " Step 2 — C-ECHO verification"
echo "════════════════════════════════════════"
"$ECHOSCU" -v -a DEMO_SCU -c STORESCP localhost "$PORT"

# ─────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════"
echo " Step 3 — Send 5 CT slices with storescu"
echo "════════════════════════════════════════"
"$STORESCU" -v -a DEMO_SCU -c STORESCP localhost "$PORT" "$FILES"/ABDOM_*.dcm

# ─────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════"
echo " Step 4 — Verify received files"
echo "════════════════════════════════════════"
RECEIVED=$(find "$RECV_DIR" -name "*.dcm" | sort)
COUNT=$(echo "$RECEIVED" | grep -c ".dcm" || true)
echo "Received $COUNT file(s):"
for f in $RECEIVED; do
  echo ""
  echo "  File: $(basename "$f")"
  "$DCMDUMP" "$f" | grep -E "^\(0010,0010\)|^\(0008,0016\)|^\(0020,0013\)|^\(0028,0010\)|^\(0028,0011\)" | sed 's/^/    /'
done

echo ""
echo "════════════════════════════════════════"
echo " Done — $COUNT/$( ls "$FILES"/ABDOM_*.dcm | wc -l | tr -d ' ') files transferred successfully"
echo "════════════════════════════════════════"
