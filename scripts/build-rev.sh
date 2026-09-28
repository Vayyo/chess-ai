#!/usr/bin/env bash
# Builds the engine at a git revision; prints the binary path (tools/bin/chess-ai-<sha>).
# Usage: scripts/build-rev.sh <rev>
set -euo pipefail

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
rev=$(git -C "$root" rev-parse --short "${1:?usage: build-rev.sh <rev>}^{commit}")
out="$root/tools/bin/chess-ai-$rev"
if [[ -x $out ]]; then
    echo "$out"
    exit 0
fi

work=$(mktemp -d)
cleanup() {
    git -C "$root" worktree remove --force "$work/src" >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

git -C "$root" worktree add --detach -q "$work/src" "$rev"
# Run from inside the worktree so its .cargo/config.toml (target-cpu) applies.
(cd "$work/src" && cargo build --release -q --target-dir "$root/target/revs") >&2
mkdir -p "$root/tools/bin"
cp "$root/target/revs/release/chess-ai" "$out"
echo "$out"
