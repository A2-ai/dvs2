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

All 8 scripts ran sequentially (not in parallel). Each script uses
\`mktemp\` for project and storage directories. File content is fresh
random bytes per rep (\`/dev/urandom\`).

| File | Files per \`dvs add\` | Order | Compression |
|------|---------------------|-------|-------------|
| \`bench_single_serial_results.csv\` | 1 | serial | zstd |
| \`bench_single_random_results.csv\` | 1 | random | zstd |
| \`bench_batch_serial_results.csv\` | 5 | serial | zstd |
| \`bench_batch_random_results.csv\` | 5 | random | zstd |
| \`bench_single_serial_no_compression_results.csv\` | 1 | serial | none |
| \`bench_single_random_no_compression_results.csv\` | 1 | random | none |
| \`bench_batch_serial_no_compression_results.csv\` | 5 | serial | none |
| \`bench_batch_random_no_compression_results.csv\` | 5 | random | none |

## Parameters

- Sizes: 1, 2, 5, 10, 50 MB
- Reps: 25 per size
- Batch size: 5 files (batch scripts)
- Storage: local filesystem (tmpdir)

## CSV format

Columns \`size_mb,rep\` are prepended by the bash script. Remaining
columns come from the dvs \`-vvv\` timing CSV:

\`\`\`
size_mb,rep,timestamp,user,dvs_version,git_commit,command,file,step,duration_ms,file_size_bytes,num_files,compression,hash_algorithm
\`\`\`

Key \`step\` values: \`hash\`, \`backend_store\`, \`add_file_total\`, \`add_total\`.
EOF

echo "Wrote $DEST/README.md"
