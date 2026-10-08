#!/bin/sh
# Render a spread of seeds as plain text for visual review.
# Usage: tools/render_seeds.sh [first_seed] [count] [rows] [cols]
set -e
cd "$(dirname "$0")/.."
SEED=${1:-1}
COUNT=${2:-12}
ROWS=${3:-24}
COLS=${4:-80}
OUT=${OUT:-/tmp/opencode/tcac-review.txt}
mkdir -p "$(dirname "$OUT")"
: > "$OUT"
i=0
while [ "$i" -lt "$COUNT" ]; do
    s=$((SEED + i))
    {
        echo "===== seed $s ($ROWS x $COLS) ====="
        LINES="$ROWS" COLUMNS="$COLS" TERM=xterm-256color \
            ./target/release/tcac -p -s "$s" \
            | sed 's/\x1b\[[0-9;]*[a-zA-Z]//g'
    } >> "$OUT"
    i=$((i + 1))
done
echo "wrote $OUT"
