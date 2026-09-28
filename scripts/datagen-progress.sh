#!/usr/bin/env bash
# Live progress of a datagen output file: positions, rate, ETA, progress bar.
# Usage: scripts/datagen-progress.sh <data.bin> [target-positions]   (default target 50000000)
# Reads only the file size, so it works with any running datagen.
set -euo pipefail

file=${1:?usage: datagen-progress.sh <data.bin> [target]}
target=${2:-50000000}
interval=2
window=30 # seconds of history for the rate estimate

declare -a hist_t=() hist_n=()
tput civis 2>/dev/null || true
trap 'tput cnorm 2>/dev/null || true; echo' EXIT

fmt_num() { printf "%'d" "$1"; }
fmt_time() {
    local s=$1
    printf "%dч %02dм %02dс" $((s / 3600)) $((s % 3600 / 60)) $((s % 60))
}

while true; do
    now=$(date +%s)
    size=$(stat -c %s "$file" 2>/dev/null || echo 0)
    n=$((size / 32))
    hist_t+=("$now")
    hist_n+=("$n")
    while ((${#hist_t[@]} > 1 && now - hist_t[0] > window)); do
        hist_t=("${hist_t[@]:1}")
        hist_n=("${hist_n[@]:1}")
    done
    dt=$((now - hist_t[0]))
    rate=0
    ((dt > 0)) && rate=$(((n - hist_n[0]) / dt))

    pct=$((n * 1000 / target))
    ((pct > 1000)) && pct=1000
    width=$(($(tput cols 2>/dev/null || echo 80) - 12))
    ((width < 10)) && width=10
    filled=$((width * pct / 1000))
    bar=$(printf "%${filled}s" "" | tr ' ' '#')$(printf "%$((width - filled))s" "" | tr ' ' '-')

    if ((rate > 0 && n < target)); then
        eta=$(fmt_time $(((target - n) / rate)))
    elif ((n >= target)); then
        eta="готово"
    else
        eta="—"
    fi

    printf "\033[H\033[2J"
    printf "chess-ai datagen: %s\n\n" "$file"
    printf "  Позиций:   %s / %s\n" "$(fmt_num "$n")" "$(fmt_num "$target")"
    printf "  Скорость:  %s поз/с (%s в час)\n" "$(fmt_num "$rate")" "$(fmt_num $((rate * 3600)))"
    printf "  Осталось:  %s\n" "$eta"
    printf "  Размер:    %s МБ\n\n" "$((size / 1048576))"
    printf "  [%s] %d.%d%%\n\n" "$bar" $((pct / 10)) $((pct % 10))
    printf "  Ctrl+C — закрыть окно (генерацию не останавливает)\n"
    sleep "$interval"
done
