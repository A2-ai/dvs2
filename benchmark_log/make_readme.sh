#!/usr/bin/env bash
set -euo pipefail

# Generate a README.md template for a benchmark results directory.
#
# Usage: ./make_readme.sh <DEST_DIR>
# Example: ./make_readme.sh benchmark_log/a7b0a984fa2566913c9cbdaf7aed351cda02753f

DEST="${1:?Usage: $0 <DEST_DIR>}"
mkdir -p "$DEST"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COMMIT=$(basename "$DEST")
SHORT=${COMMIT:0:7}
BRANCH=$(git -C "$SCRIPT_DIR" branch --contains "$COMMIT" 2>/dev/null | head -1 | sed 's/^[* ]*//' || echo "FILL_IN")
DVS_VERSION=$(dvs --version 2>/dev/null || echo "FILL_IN")
OS_VERSION=$(uname -srm)
MACHINE=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -p)
RAM=$(sysctl -n hw.memsize 2>/dev/null | awk '{printf "%d GB", $1/1073741824}' || echo "FILL_IN")

cat > "$DEST/README.md" <<EOF
# Benchmark: ${SHORT}

## Code state

Branch: \`${BRANCH}\`
Commit: \`${COMMIT}\`

FILL_IN: describe the code state here.

## Machine

- ${MACHINE}, ${RAM} RAM
- ${OS_VERSION}
- ${DVS_VERSION}

## Scripts

All 8 scripts ran sequentially (not in parallel). File content is
fresh random bytes per rep (\`/dev/urandom\`).

- **single** scripts: one file per \`dvs add\` call.
- **batch** scripts: BATCH files per \`dvs add\` call (default: REPS).
- **serial**: sizes iterate in order (1, 2, 5, 10, 50 MB), all reps per size before moving on.
- **random**: all (size, rep) pairs are shuffled before iteration (disk cache mitigation).

| File | Files per \`dvs add\` | Order | Compression |
|------|---------------------|-------|-------------|
| \`bench_single_serial_results.csv\` | 1 | serial | zstd |
| \`bench_single_random_results.csv\` | 1 | random | zstd |
| \`bench_batch_serial_results.csv\` | BATCH | serial | zstd |
| \`bench_batch_random_results.csv\` | BATCH | random | zstd |
| \`bench_single_serial_no_compression_results.csv\` | 1 | serial | none |
| \`bench_single_random_no_compression_results.csv\` | 1 | random | none |
| \`bench_batch_serial_no_compression_results.csv\` | BATCH | serial | none |
| \`bench_batch_random_no_compression_results.csv\` | BATCH | random | none |

## Parameters

- Sizes: 1, 2, 5, 10, 50 MB
- Reps: 25 per size
- Batch size: REPS files per \`dvs add\` by default (configurable via \$3)
- Storage: local filesystem (tmpdir)

## CSV format

Columns \`size_mb,rep\` are prepended by the bash script. Remaining
columns come from the dvs \`-vvv\` timing CSV:

\`\`\`
size_mb,rep,invocation_id,timestamp,user,dvs_version,git_commit,command,file,step,duration_ms,file_size_bytes,num_files,compression,hash_algorithm
\`\`\`

\`invocation_id\` is a high-precision timestamp captured once at CLI entry.
All timing records from the same \`dvs add\` call share the same \`invocation_id\`.

Key \`step\` values:

- \`hash\`: time to hash one file (blake3).
- \`backend_store\`: time to compress (if enabled) and write one file to storage.
- \`add_file_total\`: total time for one file (hash + store + metadata + audit log).
- \`add_total\`: total time for the entire \`dvs add\` invocation across all files. \`num_files\` is set on this row.
EOF

echo "Wrote $DEST/README.md"
