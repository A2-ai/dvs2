#!/usr/bin/env bash

print_eval_rscript() {
  tee /dev/stderr | Rscript -
}

install_dvs() {
  just install-cli || exit 1
  just install-rpkg || exit 1
}

cleanup_dvs_tempdirs() {
  local repo_root="${1:?repo_root is required}"
  local parent_dir

  parent_dir="$(dirname "$repo_root")"

  find "$repo_root" -maxdepth 1 -mindepth 1 \
    \( -name 'dvs_repo_*' -o -name 'dvs_fixture_*' \) \
    -exec rm -rf {} +

  find "$parent_dir" -maxdepth 1 -mindepth 1 \
    -name 'dvs_storage_*' \
    -exec rm -rf {} +
}

size_to_bytes() {
  local value="${1:?size is required}"
  local number
  local suffix

  number="${value%[KkMmGg]}"
  suffix="${value#"$number"}"

  case "$suffix" in
    "") printf '%s\n' "$number" ;;
    K|k) printf '%s\n' $((number * 1024)) ;;
    M|m) printf '%s\n' $((number * 1024 * 1024)) ;;
    G|g) printf '%s\n' $((number * 1024 * 1024 * 1024)) ;;
    *)
      printf 'Unsupported size suffix: %s\n' "$value" >&2
      return 1
      ;;
  esac
}

mkfiles() {
  { set +x; } 2>/dev/null
  local n="${1:?file count is required}"
  local size="${2:?size is required}"
  local dir="${3:?dir is required}"
  local bytes
  local width=${#n}
  local i
  local padded

  bytes="$(size_to_bytes "$size")"
  mkdir -p "$dir"

  for i in $(seq 1 "$n"); do
    padded="$(printf '%0*d' "$width" "$i")"
    head -c "$bytes" /dev/urandom > "$dir/file_${padded}.bin"
  done
  set -x
}

resolve_dataset_archetype() {
  local archetype="${1:?dataset archetype is required}"
  local candidate
  local helper_dir
  local repo_root
  local datasets_dir

  helper_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "${helper_dir}/../.." && pwd)"
  datasets_dir="${repo_root}/datasets"

  if [[ -f "$archetype" ]]; then
    printf '%s\n' "$archetype"
    return 0
  fi

  candidate="${datasets_dir}/$archetype"
  if [[ -f "$candidate" ]]; then
    printf '%s\n' "$candidate"
    return 0
  fi

  candidate="${datasets_dir}/${archetype}.csv"
  if [[ -f "$candidate" ]]; then
    printf '%s\n' "$candidate"
    return 0
  fi

  printf 'Unknown dataset archetype: %s\n' "$archetype" >&2
  return 1
}

mkdatasetfiles() {
  { set +x; } 2>/dev/null
  local n="${1:?file count is required}"
  local size="${2:?size is required}"
  local dir="${3:?dir is required}"
  local archetype="${4:-chickweight}"
  local extra_cols="${5:-10}"
  local max_int="${6:-1000}"

  local bytes dataset dataset_label size_label width padded i

  bytes="$(size_to_bytes "$size")" || return 1
  dataset="$(resolve_dataset_archetype "$archetype")" || return 1

  dataset_label="$(basename "$dataset")"
  dataset_label="${dataset_label%.csv}"
  dataset_label="${dataset_label//[^[:alnum:]_-]/_}"
  size_label="${size//[^[:alnum:]_.-]/_}"

  mkdir -p "$dir" || return 1

  width=${#n}

  for ((i = 1; i <= n; i++)); do
    padded="$(printf '%0*d' "$width" "$i")"

    awk -v target_bytes="$bytes" \
        -v extra_cols="$extra_cols" \
        -v max_int="$max_int" \
        -v seed="$i" '
      BEGIN {
        srand(seed)
      }

      function next_rand() {
        return int(rand() * max_int)
      }

      NR == 1 {
        header = $0
        for (i = 1; i <= extra_cols; i++) {
          header = header ",rand" i
        }
        next
      }

      {
        rows[++row_count] = $0
      }

      END {
        if (header == "") exit 1

        print header
        written = length(header) + 1

        if (row_count == 0) exit 0

        row_index = 0
        while (written < target_bytes) {
          row_index = (row_index % row_count) + 1
          line = rows[row_index]

          for (i = 1; i <= extra_cols; i++) {
            line = line "," next_rand()
          }

          print line
          written += length(line) + 1
        }
      }
    ' "$dataset" > "$dir/file_${dataset_label}_${size_label}_${padded}.csv" || return 1
  done
  set -x
}
