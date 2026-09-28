#!/usr/bin/env bash
# Packs a portable datagen build for another machine:
#   tools/dist/chess-ai-datagen-<cpu>.tar.gz
# The result contains the engine and scripts/datagen-remote.sh; no toolchain is
# needed on the target. Bulk of the archive is a static Rust binary.
set -euo pipefail

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cpu=${CPU:-x86-64-v3}   # AVX2 + BMI2; falls back to magic bitboards elsewhere
rev=$(git -C "$root" rev-parse --short HEAD)

# The default config pins target-cpu=native, which would not run elsewhere.
RUSTFLAGS="-C target-cpu=$cpu" CARGO_TARGET_DIR="$root/target/portable" \
    cargo build --release --quiet --manifest-path "$root/Cargo.toml"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp "$root/target/portable/release/chess-ai" "$stage/"
cp "$root/scripts/datagen-remote.sh" "$stage/"
chmod +x "$stage/datagen-remote.sh" "$stage/chess-ai"
cat >"$stage/README.txt" <<EOF
chess-ai datagen, revision $rev, built for $cpu.

  ./datagen-remote.sh <out.bin> [threads] [nodes] [seed]

Пример: ./datagen-remote.sh gen1.bin 3 5000 20260940

Данные пишутся в <out.bin> (32 байта на позицию), лог — в <out.bin>.log,
файл можно забирать и докачивать в любой момент.

Склейка данных: файлы с разными seed не пересекаются, обучение читает сразу
несколько файлов, поэтому копии можно просто сложить рядом друг с другом.

Проверка целостности после переноса (обрезает недописанный хвост):
  python3 -c "import sys,os;p=sys.argv[1];s=os.path.getsize(p);os.truncate(p,s-s%32)" <out.bin>
EOF

mkdir -p "$root/tools/dist"
archive="$root/tools/dist/chess-ai-datagen-$cpu.tar.gz"
tar -czf "$archive" -C "$stage" .
echo "$archive ($(du -h "$archive" | cut -f1))"
