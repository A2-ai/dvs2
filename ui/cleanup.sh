#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

find "$SCRIPT_DIR" -maxdepth 1 -mindepth 1 \
  \( -name 'dvs_repo_*' -o -name 'dvs_storage_*' \) \
  -exec printf 'Trashing: %s\n' '{}' \; \
  -exec trash {} +
