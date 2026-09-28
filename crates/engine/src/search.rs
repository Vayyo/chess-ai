//! Iterative-deepening PVS (negamax alpha-beta) with a transposition table,
//! null move pruning, late move reductions, quiescence search, move ordering
//! (TT move, MVV-LVA, killers, history), check extension, repetition and
//! fifty-move draws.

use std::{
    fmt::Write as _,
    sync::{
        LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use movegen::{Board, Color, Move, MoveList, PieceType};

use crate::{
    nnue::{self, Accumulators},
    tt::{Bound, TranspositionTable},
};

pub const MAX_PLY: usize = 128;
const INF: i32 = 32_001;
const MATE: i32 = 32_000;
/// Scores beyond this are forced mates.
const MATE_BOUND: i32 = MATE - MAX_PLY as i32;
/// Reserved per move for GUI/IO latency.
const MOVE_OVERHEAD_MS: u64 = 20;
/// Moves-to-go assumed in sudden-death time controls.
const DEFAULT_MOVES_TO_GO: u64 = 30;
/// Quiet-history magnitude bound; stays far below killer and capture scores.
const MAX_HISTORY: i32 = 16_384;
/// Centipawns per depth ply that a quiet move is assumed to be able to gain.
const FUTILITY_MARGIN: i32 = 100;
/// Centipawns per depth ply that the static score may overestimate the truth.
const RFP_MARGIN: i32 = 80;

/// Late-move reduction in plies, indexed by `[depth][move index]` (both capped at 63).
static LMR: LazyLock<[[i32; 64]; 64]> = LazyLock::new(|| {
    let mut table = [[0; 64]; 64];
    for (depth, row) in table.iter_mut().enumerate().skip(1) {
        for (index, r) in row.iter_mut().enumerate().skip(1) {
            *r = (0.75 + (depth as f64).ln() * (index as f64).ln() / 2.25) as i32;
        }
    }
    table
});

/// `go` parameters. Times in milliseconds; `time`/`inc` indexed by color.
#[derive(Clone, Debug, Default)]
pub struct Limits {
    pub depth: Option<u32>,
    pub nodes: Option<u64>,
    pub movetime: Option<u64>,
    pub time: [Option<u64>; 2],
    pub inc: [u64; 2],
    pub movestogo: Option<u32>,
    pub infinite: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchResult {
    pub best: Move,
    pub score: i32,
    pub nodes: u64,
    /// Depth of the last completed iteration.
    pub depth: u32,
}

/// Searches `board`; with `verbose`, prints UCI `info` lines and `bestmove`.
///
/// `history` holds hashes of the game positions before `board`, oldest first.
/// With `limits.infinite` the result is held back until `stop` is set, as UCI requires.
pub fn search(
    board: &Board,
    history: Vec<u64>,
    limits: &Limits,
    stop: &AtomicBool,
    tt: &mut TranspositionTable,
    verbose: bool,
) -> SearchResult {
    let (soft, hard) = time_budget(limits, board.side_to_move());
    tt.new_search();
    let mut searcher = Searcher::new(board, stop, hard, limits.nodes, history, tt);

    let mut root_moves = MoveList::new();
    board.generate_moves(&mut root_moves);
    let mut result =
        SearchResult { best: root_moves.first().copied().unwrap_or(Move::NULL), score: 0, nodes: 0, depth: 0 };
    if !root_moves.is_empty() {
        let max_depth = limits.depth.unwrap_or(u32::MAX).clamp(1, MAX_PLY as u32 - 1);
        for depth in 1..=max_depth {
            let score = searcher.negamax(board, depth as i32, 0, -INF, INF);
            if searcher.aborted {
                break;
            }
            result.best = searcher.pv[0][0];
            result.score = score;
            result.depth = depth;
            if verbose {
                searcher.report(depth, score);
            }
            if soft.is_some_and(|soft| searcher.start.elapsed() >= soft) {
                break;
            }
        }
    }

    if limits.infinite {
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(1));
        }
    }
    result.nodes = searcher.nodes;
    if verbose {
        println!("bestmove {}", result.best);
    }
    result
}

/// (soft, hard): stop deepening after `soft`, abort mid-iteration at `hard`.
fn time_budget(limits: &Limits, side: Color) -> (Option<Duration>, Option<Duration>) {
    if limits.infinite {
        return (None, None);
    }
    if let Some(movetime) = limits.movetime {
        let t = Duration::from_millis(movetime.saturating_sub(MOVE_OVERHEAD_MS).max(1));
        return (Some(t), Some(t));
    }
    let Some(time) = limits.time[side.index()] else { return (None, None) };
    let available = time.saturating_sub(MOVE_OVERHEAD_MS).max(1);
    let moves_to_go = limits.movestogo.map_or(DEFAULT_MOVES_TO_GO, u64::from).max(1);
    let target = available / moves_to_go + limits.inc[side.index()] * 3 / 4;
    let hard = (target * 3).min(available * 3 / 4).max(1);
    // The next iteration usually costs more than all previous ones combined.
    let soft = (target / 2).min(hard);
    (Some(Duration::from_millis(soft)), Some(Duration::from_millis(hard)))
}

struct Searcher<'a> {
    stop: &'a AtomicBool,
    tt: &'a mut TranspositionTable,
    start: Instant,
    hard: Option<Duration>,
    node_limit: Option<u64>,
    nodes: u64,
    aborted: bool,
    /// Hashes of game positions plus the current search path, for repetition detection.
    history: Vec<u64>,
    /// Triangular principal-variation table: `pv[ply][ply..pv_len[ply]]`.
    pv: Box<[[Move; MAX_PLY]; MAX_PLY]>,
    pv_len: [usize; MAX_PLY],
    /// Two most recent quiet moves that caused a beta cutoff at each ply.
    killers: [[Move; 2]; MAX_PLY],
    /// Cutoff statistics of quiet moves, `[side][from][to]`, bounded by `MAX_HISTORY`.
    quiet_history: Box<[[[i32; 64]; 64]; 2]>,
    /// `null_entered[ply]`: the node at `ply` was reached by a null move.
    null_entered: [bool; MAX_PLY],
    /// Repetition scans stop at this `history` index: positions before a null
    /// move are not reachable by legal play from inside its subtree.
    repetition_floor: usize,
    /// NNUE accumulators of the position at each ply of the current line.
    accs: Box<[Accumulators; MAX_PLY]>,
}

impl<'a> Searcher<'a> {
    fn new(
        root: &Board,
        stop: &'a AtomicBool,
        hard: Option<Duration>,
        node_limit: Option<u64>,
        history: Vec<u64>,
        tt: &'a mut TranspositionTable,
    ) -> Self {
        Searcher {
            stop,
            tt,
            start: Instant::now(),
            hard,
            node_limit,
            nodes: 0,
            aborted: false,
            history,
            pv: Box::new([[Move::NULL; MAX_PLY]; MAX_PLY]),
            pv_len: [0; MAX_PLY],
            killers: [[Move::NULL; 2]; MAX_PLY],
            quiet_history: Box::new([[[0; 64]; 64]; 2]),
            null_entered: [false; MAX_PLY],
            repetition_floor: 0,
            accs: Box::new([nnue::refresh(root); MAX_PLY]),
        }
    }
}

impl Searcher<'_> {
    fn negamax(&mut self, board: &Board, mut depth: i32, ply: usize, mut alpha: i32, beta: i32) -> i32 {
        let pv_node = beta - alpha > 1;
        self.pv_len[ply] = ply;
        if ply > 0 {
            if self.should_stop() {
                self.aborted = true;
                return 0;
            }
            if board.halfmove_clock() >= 100 || self.is_repetition(board) {
                return 0;
            }
        }
        let in_check = board.in_check();
        if in_check {
            depth += 1;
        }
        if depth <= 0 {
            return self.qsearch(board, ply, alpha, beta);
        }
        if ply >= MAX_PLY - 1 {
            return self.evaluate(board, ply);
        }
        self.nodes += 1;

        let hit = self.tt.probe(board.hash());
        if let Some(hit) = hit
            && !pv_node
            && hit.depth >= depth
        {
            let score = score_from_tt(hit.score, ply);
            let usable = match hit.bound {
                Bound::Exact => true,
                Bound::Lower => score >= beta,
                Bound::Upper => score <= alpha,
            };
            if usable {
                return score;
            }
        }
        let tt_move = hit.map_or(Move::NULL, |hit| hit.mv);
        // Static score of this node, reused by the pruning below.
        let eval = self.evaluate(board, ply);

        // Reverse futility: a static score far above beta at low depth holds up
        // almost always, so the node is cut without searching any move.
        if !pv_node && !in_check && depth <= 6 && !self.null_entered[ply] && eval - RFP_MARGIN * depth >= beta {
            return eval;
        }

        // Null move pruning: if passing still fails high on a reduced search,
        // a real move will too. Unsafe in check and in pawn endings (zugzwang).
        if !pv_node
            && !in_check
            && depth >= 3
            && !self.null_entered[ply]
            && has_non_pawn_material(board)
            && eval >= beta
        {
            let reduction = 3 + depth / 4;
            let mut child = *board;
            child.make_null_move();
            self.accs[ply + 1] = self.accs[ply];
            self.history.push(board.hash());
            let saved_floor = std::mem::replace(&mut self.repetition_floor, self.history.len());
            self.null_entered[ply + 1] = true;
            let score = -self.negamax(&child, depth - 1 - reduction, ply + 1, -beta, -beta + 1);
            self.null_entered[ply + 1] = false;
            self.repetition_floor = saved_floor;
            self.history.pop();
            if self.aborted {
                return 0;
            }
            if score >= beta {
                // A mate found after passing is not proven for real moves.
                return if score >= MATE_BOUND { beta } else { score };
            }
        }

        let mut moves = MoveList::new();
        board.generate_moves(&mut moves);
        if moves.is_empty() {
            return if board.in_check() { -MATE + ply as i32 } else { 0 };
        }
        let mut scores = self.order_scores(board, &moves, tt_move, ply);

        self.history.push(board.hash());
        let original_alpha = alpha;
        let mut best = -INF;
        let mut best_move = Move::NULL;
        let mut quiets_tried = [Move::NULL; 64];
        let mut quiet_count = 0;
        for i in 0..moves.len() {
            let mv = pick(&mut moves, &mut scores, i);
            let quiet = !mv.is_capture() && !mv.is_promotion();
            // Prune quiet moves that cannot reach alpha at this depth. The first
            // move is never pruned, so `best` always has a real value.
            if quiet && i > 0 && !pv_node && !in_check {
                if depth <= 3 && eval + FUTILITY_MARGIN * depth <= alpha {
                    continue;
                }
                if depth <= 4 && i >= 4 + (depth * depth) as usize {
                    continue;
                }
            }
            let mut child = *board;
            self.accs[ply + 1] = nnue::update(&self.accs[ply], board, mv);
            child.make_move(mv);
            // PVS: the first move gets the full window, the rest a null window
            // that only proves they are worse. LMR additionally searches late
            // quiet moves shallower; any surprise is re-searched at full depth.
            let mut score;
            if i == 0 {
                score = -self.negamax(&child, depth - 1, ply + 1, -beta, -alpha);
            } else {
                let mut reduction = 0;
                if depth >= 3 && i >= 2 && quiet && !in_check && !child.in_check() {
                    reduction = LMR[depth.min(63) as usize][i.min(63)];
                    reduction -= i32::from(pv_node);
                    // scores[i] belongs to `mv` after `pick`; killers score 79_000+.
                    reduction -= i32::from(scores[i] >= 79_000);
                    reduction = reduction.clamp(0, depth - 2);
                }
                score = -self.negamax(&child, depth - 1 - reduction, ply + 1, -alpha - 1, -alpha);
                if score > alpha && reduction > 0 {
                    score = -self.negamax(&child, depth - 1, ply + 1, -alpha - 1, -alpha);
                }
                if score > alpha && score < beta {
                    score = -self.negamax(&child, depth - 1, ply + 1, -beta, -alpha);
                }
            }
            if self.aborted {
                break;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    best_move = mv;
                    self.update_pv(ply, mv);
                    if alpha >= beta {
                        if quiet {
                            self.reward_quiet(board, mv, &quiets_tried[..quiet_count], depth, ply);
                        }
                        break;
                    }
                }
            }
            if quiet && quiet_count < quiets_tried.len() {
                quiets_tried[quiet_count] = mv;
                quiet_count += 1;
            }
        }
        self.history.pop();
        if self.aborted {
            return 0;
        }

        let bound = if best >= beta {
            Bound::Lower
        } else if alpha > original_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        self.tt.store(board.hash(), best_move, score_to_tt(best, ply), depth, bound);
        best
    }

    /// Resolves captures (all evasions when in check) until the position is quiet.
    fn qsearch(&mut self, board: &Board, ply: usize, mut alpha: i32, beta: i32) -> i32 {
        self.pv_len[ply] = ply;
        if self.should_stop() {
            self.aborted = true;
            return 0;
        }
        self.nodes += 1;
        if ply >= MAX_PLY - 1 {
            return self.evaluate(board, ply);
        }

        let mut moves = MoveList::new();
        let mut best = -INF;
        if board.in_check() {
            board.generate_moves(&mut moves);
            if moves.is_empty() {
                return -MATE + ply as i32;
            }
        } else {
            let stand_pat = self.evaluate(board, ply);
            if stand_pat >= beta {
                return stand_pat;
            }
            alpha = alpha.max(stand_pat);
            best = stand_pat;
            board.generate_noisy(&mut moves);
        }
        let mut scores = self.order_scores(board, &moves, Move::NULL, ply);

        for i in 0..moves.len() {
            let mv = pick(&mut moves, &mut scores, i);
            let mut child = *board;
            self.accs[ply + 1] = nnue::update(&self.accs[ply], board, mv);
            child.make_move(mv);
            let score = -self.qsearch(&child, ply + 1, -beta, -alpha);
            if self.aborted {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    if alpha >= beta {
                        break;
                    }
                }
            }
        }
        best
    }

    /// TT move, then captures by MVV-LVA (queen promotions among them), quiet
    /// queen promotions, killers, quiets by history; underpromotions last.
    fn order_scores(&self, board: &Board, moves: &MoveList, tt_move: Move, ply: usize) -> [i32; 256] {
        let history = &self.quiet_history[board.side_to_move().index()];
        let kind = |sq| board.piece_at(sq).map_or(PieceType::Pawn, |p| p.kind());
        let mut scores = [0; 256];
        for (score, &mv) in scores.iter_mut().zip(moves.iter()) {
            *score = if mv == tt_move {
                1_000_000
            } else if mv.promotion().is_some_and(|p| p != PieceType::Queen) {
                -100_000
            } else if mv.is_capture() {
                let victim = if mv.is_en_passant() { PieceType::Pawn } else { kind(mv.to()) };
                100_000 + 8 * victim.index() as i32 - kind(mv.from()).index() as i32
            } else if mv.is_promotion() {
                90_000
            } else if mv == self.killers[ply][0] {
                80_000
            } else if mv == self.killers[ply][1] {
                79_000
            } else {
                history[mv.from().index()][mv.to().index()]
            };
        }
        scores
    }

    /// Quiet `mv` caused a beta cutoff: make it a killer, raise its history and
    /// lower the history of the quiets searched before it.
    fn reward_quiet(&mut self, board: &Board, mv: Move, tried: &[Move], depth: i32, ply: usize) {
        if self.killers[ply][0] != mv {
            self.killers[ply][1] = self.killers[ply][0];
            self.killers[ply][0] = mv;
        }
        let bonus = (depth * depth).min(MAX_HISTORY / 8);
        let history = &mut self.quiet_history[board.side_to_move().index()];
        let updates = std::iter::once((mv, bonus)).chain(tried.iter().map(|&m| (m, -bonus)));
        for (m, delta) in updates {
            let entry = &mut history[m.from().index()][m.to().index()];
            // Gravity: steps shrink as |entry| nears MAX_HISTORY, so it never exceeds it.
            *entry += delta - *entry * delta.abs() / MAX_HISTORY;
        }
    }

    /// Static evaluation of `board`, the position at `ply` of the current line.
    #[inline]
    fn evaluate(&self, board: &Board, ply: usize) -> i32 {
        nnue::evaluate(&self.accs[ply], board.side_to_move())
    }

    #[inline]
    fn should_stop(&mut self) -> bool {
        if self.aborted || self.node_limit.is_some_and(|limit| self.nodes >= limit) {
            return true;
        }
        self.nodes & 1023 == 0
            && (self.stop.load(Ordering::Relaxed) || self.hard.is_some_and(|hard| self.start.elapsed() >= hard))
    }

    /// Same position with the same side to move since the last irreversible move.
    fn is_repetition(&self, board: &Board) -> bool {
        let hash = board.hash();
        let window = (board.halfmove_clock() as usize).min(self.history.len() - self.repetition_floor);
        // history[len - 1] is the parent (other side to move); same side is every second entry.
        (2..=window).step_by(2).any(|back| self.history[self.history.len() - back] == hash)
    }

    fn update_pv(&mut self, ply: usize, mv: Move) {
        let child_len = self.pv_len[ply + 1];
        let (head, tail) = self.pv.split_at_mut(ply + 1);
        head[ply][ply] = mv;
        head[ply][ply + 1..child_len].copy_from_slice(&tail[0][ply + 1..child_len]);
        self.pv_len[ply] = child_len.max(ply + 1);
    }

    fn report(&self, depth: u32, score: i32) {
        let elapsed = self.start.elapsed();
        let nps = (self.nodes as f64 / elapsed.as_secs_f64().max(1e-6)) as u64;
        let mut line = format!(
            "info depth {depth} score {} nodes {} nps {nps} hashfull {} time {} pv",
            uci_score(score),
            self.nodes,
            self.tt.hashfull(),
            elapsed.as_millis()
        );
        for mv in &self.pv[0][..self.pv_len[0]] {
            write!(line, " {mv}").unwrap();
        }
        println!("{line}");
    }
}

/// Score proves a forced mate for either side.
pub fn is_mate_score(score: i32) -> bool {
    score.abs() >= MATE_BOUND
}

fn uci_score(score: i32) -> String {
    if score.abs() >= MATE_BOUND {
        let moves = (MATE - score.abs() + 1) / 2;
        format!("mate {}", if score > 0 { moves } else { -moves })
    } else {
        format!("cp {score}")
    }
}

/// Side to move owns a piece other than pawns and king.
fn has_non_pawn_material(board: &Board) -> bool {
    let pawns_and_kings = board.pieces(PieceType::Pawn) | board.pieces(PieceType::King);
    (board.color(board.side_to_move()) & !pawns_and_kings).any()
}

/// Mate scores are stored relative to the node, not the root, so a TT hit at
/// another ply still reports the right distance to mate.
fn score_to_tt(score: i32, ply: usize) -> i32 {
    if score >= MATE_BOUND {
        score + ply as i32
    } else if score <= -MATE_BOUND {
        score - ply as i32
    } else {
        score
    }
}

fn score_from_tt(score: i32, ply: usize) -> i32 {
    if score >= MATE_BOUND {
        score - ply as i32
    } else if score <= -MATE_BOUND {
        score + ply as i32
    } else {
        score
    }
}

/// Selection sort step: moves the best-scored remaining move to `index`.
#[inline]
fn pick(moves: &mut [Move], scores: &mut [i32; 256], index: usize) -> Move {
    let mut best = index;
    for j in index + 1..moves.len() {
        if scores[j] > scores[best] {
            best = j;
        }
    }
    moves.swap(index, best);
    scores.swap(index, best);
    moves[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(fen: &str, depth: u32) -> SearchResult {
        let board = Board::from_fen(fen).unwrap();
        let stop = AtomicBool::new(false);
        let mut tt = TranspositionTable::new(1);
        search(&board, Vec::new(), &Limits { depth: Some(depth), ..Limits::default() }, &stop, &mut tt, false)
    }

    #[test]
    fn finds_mate_in_one() {
        // Deeper than needed: TT hits from later iterations must keep the mate distance.
        let r = run("r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4", 6);
        assert_eq!(r.best.to_string(), "h5f7");
        assert_eq!(r.score, MATE - 1);
    }

    #[test]
    fn stalemate_returns_null_move_and_draw() {
        let r = run("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1", 4);
        assert_eq!((r.best, r.score), (Move::NULL, 0));
    }

    #[test]
    fn detects_repetition_through_game_history() {
        // Kings shuffle back to the start; playing Kg1-h1 again repeats a position from the game.
        let mut board = Board::from_fen("6k1/5q2/8/8/8/8/8/6K1 w - - 0 1").unwrap();
        let mut history = Vec::new();
        for m in ["g1h1", "g8h8", "h1g1", "h8g8"] {
            history.push(board.hash());
            let mv = board.find_uci_move(m).unwrap();
            board.make_move(mv);
        }
        let stop = AtomicBool::new(false);
        let mut tt = TranspositionTable::new(1);
        let mut s = Searcher::new(&board, &stop, None, None, history, &mut tt);
        s.history.push(board.hash());
        let mut child = board;
        child.make_move(board.find_uci_move("g1h1").unwrap());
        assert!(s.is_repetition(&child));
    }

    #[test]
    fn time_budget_stays_within_clock() {
        let limits = Limits { time: [Some(1_000), None], movestogo: Some(1), ..Limits::default() };
        let (soft, hard) = time_budget(&limits, Color::White);
        assert!(soft.unwrap() <= hard.unwrap());
        assert!(hard.unwrap() < Duration::from_millis(1_000));
    }
}
