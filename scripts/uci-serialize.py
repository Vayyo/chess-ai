#!/usr/bin/env python3
"""Serializes UCI traffic for engines that cannot handle pipelined commands.

Modern match managers (fastchess, cutechess) send `isready` while a search is
running and may send the next `position` before `bestmove` arrives. Engines from
the 2000s (Stockfish 1.x/2.x) mishandle that: they never answer the outstanding
`go` in time and lose on the clock.

This proxy forwards every command immediately except `position` and `go`, which
are held back until the engine has answered the previous search with `bestmove`.

Usage: uci-serialize.py <engine> [engine args...]
"""

import subprocess
import sys
import threading

if len(sys.argv) < 2:
    sys.exit("usage: uci-serialize.py <engine> [engine args...]")

proc = subprocess.Popen(sys.argv[1:], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
lock = threading.Lock()
held: list[str] = []
searching = False


def send(command: str) -> None:
    proc.stdin.write(command)
    proc.stdin.flush()


def flush_held() -> None:
    """Forward the held `position`/`go` after the engine reported `bestmove`."""
    global searching
    while True:
        with lock:
            if not held:
                return
            command = held.pop(0)
            if command.split(" ", 1)[0] == "go":
                searching = True
        send(command)


def pump_engine() -> None:
    global searching
    for line in proc.stdout:
        sys.stdout.write(line)
        sys.stdout.flush()
        if line.startswith("bestmove"):
            with lock:
                searching = False
            flush_held()
    proc.wait()


threading.Thread(target=pump_engine, daemon=True).start()

for line in sys.stdin:
    verb = line.split(" ", 1)[0]
    if verb in ("position", "go"):
        with lock:
            if searching:
                held.append(line)
                continue
            if verb == "go":
                searching = True
    send(line)
    if verb == "quit":
        break
