# SPRT log

Every functional change is tested against its parent with `scripts/sprt.sh`.
Elo is logistic, ± is the 95% interval; bounds `[elo0, elo1]`, alpha = beta = 0.05.
`NODES=50000` games are node-limited (load-independent, blind to speed changes).

| Commit | Change | Conditions | Games | Elo | Result |
|---|---|---|---|---|---|
| `027cf2f` | Transposition table + PVS | NODES=50000, [0, 5] | 488 | +159.1 ± 29.0 | H1 |
| `035b795` | Killer moves + history heuristic | NODES=50000, [0, 5] | 436 | +190.9 ± 33.2 | H1 |
| `0147111` | Null move pruning | NODES=50000, [0, 5] | 1040 | +58.7 ± 17.2 | H1 |
| `b70db23` | Late move reductions | NODES=50000, [0, 5] | 840 | +76.1 ± 19.6 | H1 |

Time-control check of the whole step 2 against step 1 (`2641610` → `83fe25e`), TC=10+0.1,
stopped early as conclusive under heavy machine load: 70 games, +62 −1 =7 (≈ +470 Elo).

| Commit | Change | Conditions | Games | Elo | Result |
|---|---|---|---|---|---|
| `88a2ca4` | NNUE replaces the PST evaluation | NODES=50000, [0, 5] | 332 | +363.3 ± 49.2 | H1 |
| `88a2ca4` | Same, time control | TC=10+0.1, [0, 5] | 80 | +70 −2 =8 (92.5%) ≈ +430 | stopped early, no time losses |

`bffa928` (AVX2 kernels) is a pure speed change: identical node signature, no SPRT.

| Commit | Change | Conditions | Games | Elo | Result |
|---|---|---|---|---|---|
| `660f100` | Futility and late move pruning | NODES=50000, [0, 5] | 1765 | +53.8% score ≈ +26.5 ± 8 | stopped early, significant |
| `edd8245` | Reverse futility pruning | NODES=50000, [0, 5] | 621 | +59.3% score ≈ +66 | stopped early, significant |

Network: `nets/current.bin`, trained by `trainer/` (bullet) for 20 epochs over the
51.9M-position `gen0` self-play dataset (games played and labelled by the PST engine),
final loss 0.0083, 768x256x2 SCReLU.
