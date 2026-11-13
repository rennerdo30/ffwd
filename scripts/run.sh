#!/usr/bin/env bash
# Build and start ffwd from this checkout (macOS, Linux).
#
#   scripts/run.sh                 open this repository
#   scripts/run.sh ~/some/repo     open another repository
#   scripts/run.sh --release       optimized build (the real speed)
#   scripts/run.sh --pull          update to the latest main first (fast-forward only)
#   scripts/run.sh --trace         print startup and present timings when ffwd closes
set -euo pipefail
cd "$(dirname "$0")/.."

pull=0 release=() repo="$PWD" args=()
for arg in "$@"; do
  case "$arg" in
    --pull) pull=1 ;;
    --release) release=(--release) ;;
    --trace) export FFWD_TRACE=1 ;;
    -*) args+=("$arg") ;;
    *) repo="$arg" ;;
  esac
done

if [ "$pull" = 1 ]; then git pull --ff-only origin main; fi
cpus=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)
jobs=$(( cpus / 2 > 0 ? cpus / 2 : 1 )); [ "$jobs" -gt 8 ] && jobs=8

echo "Building ffwd ${release[*]:-}..."
cargo build "${release[@]}" -j "$jobs" --quiet -p ffwd
exec cargo run "${release[@]}" -j "$jobs" --quiet -p ffwd -- "$repo" "${args[@]}"
