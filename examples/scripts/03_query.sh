#!/usr/bin/env bash
# 03_query.sh — findscu showcase
#
# Starts a Storage SCP, sends the 5 ABDOM files, then queries with findscu.
# (findscu requires a Query/Retrieve SCP; real-world usage would target a PACS.)
#
# Note: storescp in dicom-toolkit-rs only handles C-STORE, not C-FIND.  This script
# shows the findscu command-line interface against a real external SCP.
# If you have Orthanc or another PACS running locally, adjust HOST/PORT below.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/../.."
FINDSCU="$ROOT/target/debug/findscu"
STORESCU="$ROOT/target/debug/storescu"
STORESCP="$ROOT/target/debug/storescp"
FILES="$SCRIPT_DIR/../testfiles"

if [[ ! -x "$FINDSCU" ]]; then
  echo "Binary not found — run:  cargo build --bins"
  exit 1
fi

# ─────────────────────────────────────────────
echo "════════════════════════════════════════════════════════════"
echo " findscu — C-FIND query examples"
echo "════════════════════════════════════════════════════════════"
echo ""
echo " The following commands show common C-FIND patterns."
echo " Set PACS_HOST / PACS_PORT to a running Query/Retrieve SCP."
echo ""

PACS_HOST="${PACS_HOST:-localhost}"
PACS_PORT="${PACS_PORT:-4242}"

echo "────────────────────────────────────────────────────────────"
echo " Example 1: Find all studies (wildcard)"
echo "────────────────────────────────────────────────────────────"
echo "Command:"
echo "  findscu -L STUDY -k '0008,0052=STUDY' -k '0010,0010=' \\"
echo "          $PACS_HOST $PACS_PORT"
echo ""

echo "────────────────────────────────────────────────────────────"
echo " Example 2: Find by patient name (wildcard prefix)"
echo "────────────────────────────────────────────────────────────"
echo "Command:"
echo "  findscu -L STUDY -k '0010,0010=AVE*' $PACS_HOST $PACS_PORT"
echo ""

echo "────────────────────────────────────────────────────────────"
echo " Example 3: Find at SERIES level for a specific StudyUID"
echo "────────────────────────────────────────────────────────────"
echo "Command:"
echo "  findscu -L SERIES -k '0020,000D=<StudyInstanceUID>' $PACS_HOST $PACS_PORT"
echo ""

echo "────────────────────────────────────────────────────────────"
echo " Example 4: Find CT images in a date range"
echo "────────────────────────────────────────────────────────────"
echo "Command:"
echo "  findscu -L STUDY -k '0008,0060=CT' -k '0008,0020=19960101-19971231' \\"
echo "          $PACS_HOST $PACS_PORT"
echo ""

# If PACS is reachable (skip by default so this script always exits 0)
if [[ "${RUN_LIVE:-0}" == "1" ]]; then
  echo "════════════════════════════════════════════════════════════"
  echo " LIVE query against $PACS_HOST:$PACS_PORT"
  echo "════════════════════════════════════════════════════════════"
  "$FINDSCU" -v \
    -a FINDSCU -c ANY-SCP \
    -L STUDY \
    -k "0010,0010=" \
    "$PACS_HOST" "$PACS_PORT"
else
  echo "(Set RUN_LIVE=1 to execute queries against $PACS_HOST:$PACS_PORT)"
fi
