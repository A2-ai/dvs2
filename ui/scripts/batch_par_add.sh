#!/usr/bin/env bash
set -euo pipefail
LOGFILE="${0##*/}"
LOGFILE="${LOGFILE%.*}.log"
exec > >(tee "$LOGFILE") 2>&1

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Archetypes:
#   n     size    description
#   10    1e6     few large files (big genomics tables)
#   100   1e5     moderate count, moderate size (typical clinical datasets)
#   1000  1e4     many small-medium files (batch simulation outputs)
#   10000 1e2     lots of tiny files (parameter sweeps / configs)

CONFIGS=(
  "10    1e6"
  "100   1e5"
  "1000  1e4"
  "10000 1e2"
)

echo "==========================================="
echo "  Batch benchmark: par_add.sh"
echo "==========================================="
echo

for cfg in "${CONFIGS[@]}"; do
  read -r n size <<< "$cfg"
  echo "###############################################"
  echo "### n=$n  size=$size"
  echo "###############################################"
  echo
  bash "$SCRIPT_DIR/par_add.sh" "$n" "$size"
  echo
done

# collect all summaries at the end
echo ""
echo "=================================================="
echo "  Combined Summary"
echo "=================================================="
for cfg in "${CONFIGS[@]}"; do
  read -r n size <<< "$cfg"
  logfile="par_add_n${n}_s${size}.log"
  if [ -f "$logfile" ]; then
    grep -A100 "Timing Summary" "$logfile" | head -20
    echo
  fi
done
