#!/usr/bin/env bash
# Calibration match against an engine with a published rating (e.g. a CCRL-listed
# build). Unlike Stockfish's UCI_Elo, such an opponent plays at its real strength.
# Usage: scripts/calibrate.sh <opponent-cmd> [games] [tc|st]
# Env: CONCURRENCY (5), ENGINE (target/release/chess-ai)
#   tc=60+0.6   time control (minutes:seconds+increment); old engines may overspend
#   st=2        fixed seconds per move; immune to time-management quirks
set -euo pipefail

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
opponent=${1:?usage: calibrate.sh <opponent-cmd> [games] [tc|st]}
games=${2:-100}
limit=${3:-10+0.1}
engine=${ENGINE:-$root/target/release/chess-ai}
read -r -a parts <<<"$opponent"
name=${OPPONENT_NAME:-$(basename "${parts[-1]}")}
args=()
if ((${#parts[@]} > 1)); then
    args=("args=${parts[*]:1}")
fi
case $limit in
    tc=* | st=*) timing=("$limit") ;;
    *) timing=("tc=$limit") ;;
esac

mkdir -p "$root/tools/gauntlet"
# Old engines overshoot the move deadline (Stockfish 2.0.1 by ~225 ms), which
# fastchess treats as a loss unless a margin is allowed.
margin=(timemargin="${MARGIN:-1000}")
pgn="$root/tools/gauntlet/$name-$(date +%Y%m%d-%H%M%S).pgn"
printf '%s\n' "${CCRL:-}" >"$pgn.rating"
printf '%s\n' "$opponent" >"$pgn.opponent"
cd "$root/tools/gauntlet"
exec "$root/tools/fastchess/fastchess" \
    -engine cmd="$engine" name=chess-ai \
    -engine cmd="${parts[0]}" "${args[@]}" name="$name" \
    -each "${timing[@]}" "${margin[@]}" option.Hash=16 \
    -openings file="$root/tools/8moves_v3.pgn" format=pgn order=random \
    -repeat -rounds "$((games / 2))" -concurrency "${CONCURRENCY:-5}" \
    -pgnout file="$pgn"
