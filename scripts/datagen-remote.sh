#!/bin/sh
# Self-play data generation on a remote machine (no toolchain needed).
# Usage: ./datagen-remote.sh <out.bin> [threads] [nodes] [seed]
set -e

here=$(cd "$(dirname "$0")" && pwd)
out=${1:?usage: datagen-remote.sh <out.bin> [threads] [nodes] [seed]}
threads=${2:-$(nproc 2>/dev/null || echo 2)}
nodes=${3:-5000}
seed=${4:-1}

if [ -f "$out.pid" ] && kill -0 "$(cat "$out.pid")" 2>/dev/null; then
    echo "уже запущено, pid $(cat "$out.pid")" >&2
    exit 1
fi

nohup "$here/chess-ai" datagen "$out" --threads "$threads" --nodes "$nodes" --seed "$seed" \
    --positions 1000000000 >"$out.log" 2>&1 &
echo $! >"$out.pid"
echo "запущено: pid $(cat "$out.pid"), лог $out.log"
echo "прогресс: tail -f $out.log"
