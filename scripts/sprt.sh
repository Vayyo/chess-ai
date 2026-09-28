#!/usr/bin/env bash
# SPRT of NEW against BASE with fastchess.
# Usage: scripts/sprt.sh <base-rev> [new-rev]   (without new-rev: current working tree)
# Env: TC (8+0.08), NODES (unset), CONCURRENCY (4), HASH (16), ELO0 (0), ELO1 (5)
# NODES=N plays node-limited games instead: immune to machine load, but blind to speed changes.
# Needs tools/fastchess/fastchess and tools/8moves_v3.pgn (see README).
set -euo pipefail

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
base=$("$root/scripts/build-rev.sh" "${1:?usage: sprt.sh <base-rev> [new-rev]}")
if [[ $# -ge 2 ]]; then
    new=$("$root/scripts/build-rev.sh" "$2")
else
    (cd "$root" && cargo build --release -q)
    # Snapshot so rebuilding during the match cannot swap the binary.
    new="$root/tools/bin/chess-ai-worktree"
    mkdir -p "$root/tools/bin"
    cp "$root/target/release/chess-ai" "$new"
fi

mkdir -p "$root/tools/sprt"
# fastchess autosaves config.json into its working directory.
cd "$root/tools/sprt"
if [[ -n ${NODES:-} ]]; then
    # The clock only guards against hangs; the node limit decides.
    limit=(tc=600+1 nodes="$NODES")
else
    limit=(tc="${TC:-8+0.08}")
fi
exec "$root/tools/fastchess/fastchess" \
    -engine cmd="$new" name=new \
    -engine cmd="$base" name=base \
    -each "${limit[@]}" option.Hash="${HASH:-16}" \
    -openings file="$root/tools/8moves_v3.pgn" format=pgn order=random \
    -repeat -rounds 50000 -concurrency "${CONCURRENCY:-4}" \
    -sprt elo0="${ELO0:-0}" elo1="${ELO1:-5}" alpha=0.05 beta=0.05 \
    -pgnout file="$root/tools/sprt/$(date +%Y%m%d-%H%M%S).pgn"
