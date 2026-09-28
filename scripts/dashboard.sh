#!/usr/bin/env bash
# Live dashboard: calibration matches, training-data progress, running processes.
# Usage: scripts/dashboard.sh [positions-target]
set -uo pipefail

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
target=${1:-50000000}
state="${TMPDIR:-/tmp}/chess-ai-dashboard.state"

bar() { # $1 percent 0..100, $2 width
    local filled=$(( $2 * $1 / 100 )) i
    for ((i = 0; i < $2; i++)); do
        ((i < filled)) && printf '#' || printf '-'
    done
}

# "games wins losses draws" for our engine against the opponent in $1.
tally() {
    awk '
        /^\[White "/ { w = $0; sub(/^\[White "/, "", w); sub(/".*/, "", w) }
        /^\[Result "/ {
            r = $0; sub(/^\[Result "/, "", r); sub(/".*/, "", r)
            if (r == "1/2-1/2") d++
            else if ((w == "chess-ai" && r == "1-0") || (w != "chess-ai" && r == "0-1")) win++
            else loss++
        }
        END { printf "%d %d %d %d", win + loss + d, win, loss, d }
    ' "$1" 2>/dev/null
}

tput civis 2>/dev/null || true
trap 'tput cnorm 2>/dev/null || true; echo' EXIT

while true; do
    printf '\033[H\033[2J'
    printf '  \033[1mchess-ai — статистика\033[0m   %s\n' "$(date +%H:%M:%S)"
    printf '  ──────────────────────────────────────────────────────────\n'

    echo
    printf '  \033[1mКАЛИБРОВКА (против движков с рейтингом CCRL)\033[0m\n'
    found=0
    for pgn in $(ls -t "$root"/tools/gauntlet/*.pgn 2>/dev/null | head -3); do
        opp=""
        [ -f "$pgn.opponent" ] && opp=$(basename "$(head -1 "$pgn.opponent")")
        if [ -z "$opp" ]; then
            opp=$(grep -m2 -E '^\[Engine(White|Black)Name' "$pgn" \
                | sed 's/^\[Engine[^ ]* *"//; s/"\]$//' | grep -v '^chess-ai' | head -1)
        fi
        [ -n "$opp" ] || continue
        read -r g w l d <<<"$(tally "$pgn")"
        [ "${g:-0}" -gt 0 ] || continue
        found=1
        score=$(awk -v w="$w" -v d="$d" -v g="$g" 'BEGIN { printf "%.1f", 100 * (w + d / 2) / g }')
        elo=$(head -1 "$pgn.rating" 2>/dev/null)
        started=$(basename "$pgn" | sed 's/.*-\(..\)\(..\)\(..\)\.pgn/\1:\2/')
        printf '    %-28s CCRL %-5s (матч с %s)\n' "$opp" "${elo:-—}" "$started"
        printf '    партий %-4s очки %5s%%   +%s =%s -%s\n' "$g" "$score" "$w" "$d" "$l"
        printf '    [%s]\n' "$(bar "${score%%.*}" 40)"
    done
    ((found)) || printf '    (матчей нет)\n'

    echo
    printf '  \033[1mДАННЫЕ ДЛЯ ОБУЧЕНИЯ\033[0m\n'
    for file in "$root"/data/*.bin; do
        [ -f "$file" ] || continue
        size=$(stat -c %s "$file")
        n=$((size / 32))
        name=$(basename "$file")
        if [ -f "$state.$name" ]; then
            read -r prev_n prev_t <"$state.$name"
        else
            prev_n=0
            prev_t=$(date +%s)
        fi
        now=$(date +%s)
        dt=$((now - prev_t))
        rate=0
        ((dt > 0 && n > prev_n)) && rate=$(((n - prev_n) / dt))
        printf '%d %s\n' "$n" "$now" >"$state.$name"
        pct=$((n * 100 / target))
        ((pct > 100)) && pct=100
        if ((rate > 0)); then
            eta=$(awk -v r="$((target - n))" -v v="$rate" 'BEGIN { t = r / v; printf "%dч %02dм", t / 3600, (t % 3600) / 60 }')
        else
            eta="пауза"
        fi
        printf '    %-12s %12s позиций  %3d%%  %s поз/с  осталось %s\n' \
            "$name" "$n" "$pct" "$rate" "$eta"
        printf '    [%s]\n' "$(bar "$pct" 40)"
    done

    echo
    printf '  \033[1mПРОЦЕССЫ\033[0m\n'
    printf '    матч fastchess: %s    генерация: %s    тренер: %s\n' \
        "$(pgrep -c fastchess 2>/dev/null || true)" \
        "$(pgrep -f 'chess-ai.*datagen' >/dev/null 2>&1 && echo идёт || echo остановлена)" \
        "$(pgrep -c trainer 2>/dev/null || true)"
    echo
    printf '  загрузка: %s   Ctrl+C — закрыть окно\n' "$(cut -d' ' -f1-3 /proc/loadavg)"
    sleep 2
done
