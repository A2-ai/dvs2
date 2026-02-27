#!/usr/bin/env bash
set -euo pipefail
LOGFILE="${0##*/}" 
LOGFILE="${LOGFILE%.*}.log" # remove last extension
exec > >(tee "$LOGFILE") 2>&1

run() { echo "$ $*"; "$@"; echo; }

TMPDIR="$(mktemp -d)"
# trap 'rm -rf "$TMPDIR"' EXIT # remove everything when exit

cd "$TMPDIR" 
mkdir projectA
mkdir storage

STORAGE="$TMPDIR/storage"
PROJECT_A="$TMPDIR/projectA"

run cd "$PROJECT_A"
run git init
run mkdir data
run dvs init "$STORAGE"
# echo "$PWD"
# run ls -la
run tree "$TMPDIR"
run R -q -e 'write.csv(head(Theoph), "data/theoph-head.csv")'
run dvs add data/theoph-head.csv --message "adding file within-tree"
run cat "$STORAGE"/audit.log.jsonl

# adding file outside of project tree

run R -q --vanilla -e "write.csv(head(Theoph), \"$TMPDIR/theoph-head.csv\")"

run dvs add "$TMPDIR"/theoph-head.csv --message "addding a file out-of-tree"

exit 1
