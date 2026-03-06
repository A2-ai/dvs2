#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BASE_DIR="${1:-$HOME/tmp/dvs_bench_$(date +%Y%m%d_%H%M%S)}"
STORAGE_BASE="${2:-$HOME/tmp/dvs_bench_storage}"

echo "========================================"
echo "DVS Benchmark"
echo "Base:    $BASE_DIR"
echo "Storage: $STORAGE_BASE"
echo "Date:    $(date)"
echo "========================================"

cleanup() {
  echo ""
  echo "Cleaning up $BASE_DIR and $STORAGE_BASE ..."
  rm -rf "$BASE_DIR" "$STORAGE_BASE"
}
trap cleanup EXIT

mkdir -p "$BASE_DIR"

# Generate data once into script_dir/data/
DATA_DIR="$SCRIPT_DIR/data"
if ! ls "$DATA_DIR"/data_*mb.tab &>/dev/null; then
  echo ""
  echo "--- Generating data ---"
  rm -rf "$DATA_DIR"
  Rscript "$SCRIPT_DIR/generate_data.R" "$DATA_DIR"
fi

write_config() {
  local config_file="$1"
  {
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "hostname: $(hostname)"
    echo "os: $(uname -s)"
    echo "os_version: $(uname -r)"
    echo "arch: $(uname -m)"

    # CPU
    if [ "$(uname -s)" = "Darwin" ]; then
      echo "cpu: $(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)"
      echo "cpu_cores: $(sysctl -n hw.ncpu 2>/dev/null || echo unknown)"
      echo "ram_bytes: $(sysctl -n hw.memsize 2>/dev/null || echo unknown)"
    else
      echo "cpu: $(lscpu 2>/dev/null | awk -F: '/Model name/ {gsub(/^ +/,"",$2); print $2; exit}' || echo unknown)"
      echo "cpu_cores: $(nproc 2>/dev/null || echo unknown)"
      echo "ram_bytes: $(awk '/MemTotal/ {print $2 * 1024}' /proc/meminfo 2>/dev/null || echo unknown)"
    fi

    # Disk / filesystem
    if [ "$(uname -s)" = "Darwin" ]; then
      echo "filesystem: $(diskutil info / 2>/dev/null | awk -F: '/File System Personality/ {gsub(/^ +/,"",$2); print $2}' || echo unknown)"
      echo "disk_protocol: $(diskutil info / 2>/dev/null | awk -F: '/Protocol/ {gsub(/^ +/,"",$2); print $2}' || echo unknown)"
    else
      echo "filesystem: $(df -T / 2>/dev/null | awk 'NR==2 {print $2}' || echo unknown)"
      local rotational
      rotational=$(lsblk -dno ROTA "$(df / 2>/dev/null | awk 'NR==2 {print $1}')" 2>/dev/null || echo "")
      if [ "$rotational" = "0" ]; then echo "disk_type: ssd"
      elif [ "$rotational" = "1" ]; then echo "disk_type: hdd"
      else echo "disk_type: unknown"
      fi
    fi

    # R and dvs versions
    echo "r_version: $(Rscript -e 'cat(R.version.string)' 2>/dev/null || echo unknown)"
    echo "dvs_cli_version: $(dvs --version 2>/dev/null || echo unknown)"
    echo "dvs1_version: $(Rscript -e 'cat(as.character(packageVersion("dvs1")))' 2>/dev/null || echo unknown)"

    # Git
    echo "commit: $(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
  } > "$config_file"
}

copy_data() {
  local mode="$1"
  echo ""
  echo "--- Copying data for $mode ---"
  local pids=()
  for src in "$DATA_DIR"/data_*mb.tab; do
    [ -f "$src" ] || continue
    local name
    name=$(basename "$src" .tab)
    local padded=${name#data_}

    local proj="$BASE_DIR/$mode/bench_${padded}"
    mkdir -p "$proj"
    cp "$src" "$proj/" &
    pids+=($!)
    echo "  $name -> $proj (background)"
  done
  for pid in "${pids[@]}"; do
    wait "$pid"
  done
  echo "Copies complete."
}

# --- Benchmark dvs1 (old, R package) ---
if Rscript -e 'if (!requireNamespace("dvs1", quietly = TRUE)) quit(status = 1)' 2>/dev/null; then
  echo ""
  echo "========================================"
  echo "dvs1 R package found — benchmarking old dvs"
  echo "========================================"
else
  echo ""
  echo "--- dvs1 not installed, installing via just install-dvs1 ---"
  (cd "$REPO_ROOT" && just install-dvs1)
fi
RESULTS_STEM="results_$(date +%Y%m%d_%H%M%S)"
RESULTS_CSV="$SCRIPT_DIR/${RESULTS_STEM}.csv"
rm -f "$RESULTS_CSV"

copy_data rpkg
Rscript "$SCRIPT_DIR/bench.R" rpkg "$BASE_DIR" "$STORAGE_BASE" "$RESULTS_CSV"

# --- Benchmark dvs (new, CLI) ---
echo ""
echo "========================================"
echo "Installing dvs CLI via just install-cli"
echo "========================================"
(cd "$REPO_ROOT" && just install-cli)

copy_data cli
Rscript "$SCRIPT_DIR/bench.R" cli "$BASE_DIR" "$STORAGE_BASE" "$RESULTS_CSV"

# Write system metadata
write_config "$SCRIPT_DIR/${RESULTS_STEM}.config"

echo ""
echo "========================================"
echo "Results:  $RESULTS_CSV"
echo "Metadata: $SCRIPT_DIR/${RESULTS_STEM}.config"
echo "========================================"
