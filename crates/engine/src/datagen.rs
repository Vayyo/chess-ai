//! `chess-ai datagen`: multi-threaded self-play producing NNUE training data
//! in bullet's 32-byte `ChessBoard` format.
//!
//! Each game starts from 8–9 random plies (rejected if unbalanced), then both
//! sides play fixed-node searches. Quiet, non-check positions are recorded
//! with the search score and labelled with the final result.

use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use movegen::{Board, Color, MoveList, PieceType, Square};
use parking_lot::Mutex;

use crate::{
    search::{self, Limits},
    tt::TranspositionTable,
};

pub const USAGE: &str = "usage: chess-ai datagen <output.bin> [--threads N] [--positions N] [--nodes N] [--seed N]";

/// Size of one bullet `ChessBoard` record.
const RECORD_BYTES: u64 = 32;
const RANDOM_PLIES: u32 = 8;
/// Openings the engine scores beyond this (centipawns) are discarded.
const MAX_OPENING_SCORE: i32 = 1_000;
/// Adjudicate a win after this many consecutive plies at or beyond `WIN_SCORE` for one side.
const WIN_SCORE: i32 = 2_000;
const WIN_PLIES: i32 = 4;
/// Adjudicate a draw after `DRAW_PLIES` consecutive plies within ±`DRAW_SCORE`, from `DRAW_MIN_PLY` on.
const DRAW_SCORE: i32 = 10;
const DRAW_PLIES: u32 = 8;
const DRAW_MIN_PLY: u32 = 60;
const MAX_GAME_PLIES: u32 = 400;
const TT_MB: usize = 8;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);

/// Game result from white's point of view, as bullet encodes it for white to move.
const BLACK_WINS: u8 = 0;
const DRAW: u8 = 1;
const WHITE_WINS: u8 = 2;

pub struct Config {
    pub out: PathBuf,
    pub threads: usize,
    pub positions: u64,
    pub nodes: u64,
    pub seed: u64,
}

impl Config {
    pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Config, String> {
        let out = args.next().ok_or("missing output path")?;
        let default_threads = thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).max(1));
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        let mut config =
            Config { out: out.into(), threads: default_threads, positions: 1_000_000, nodes: 5_000, seed: nanos };
        while let Some(flag) = args.next() {
            let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
            let number = || value.parse::<u64>().map_err(|_| format!("{flag}: `{value}` is not a number"));
            match flag.as_str() {
                "--threads" => config.threads = number()?.max(1) as usize,
                "--positions" => config.positions = number()?,
                "--nodes" => config.nodes = number()?.max(1),
                "--seed" => config.seed = number()?,
                _ => return Err(format!("unknown flag {flag}")),
            }
        }
        Ok(config)
    }
}

#[derive(Default)]
struct Stats {
    games: AtomicU64,
    positions: AtomicU64,
    /// Indexed by white-relative result.
    results: [AtomicU64; 3],
}

pub fn run(config: &Config) -> io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(&config.out)?;
    // A killed run can leave a torn record at the end; appending must restart on a record boundary.
    let len = file.metadata()?.len();
    if len % RECORD_BYTES != 0 {
        file.set_len(len - len % RECORD_BYTES)?;
    }
    let existing = file.metadata()?.len() / RECORD_BYTES;
    println!(
        "datagen: {} threads, {} nodes/move, target {} positions, seed {}, appending to {} ({existing} existing)",
        config.threads,
        config.nodes,
        config.positions,
        config.seed,
        config.out.display()
    );

    let file = Mutex::new(file);
    let stats = Stats::default();
    let finished = AtomicUsize::new(0);
    let start = Instant::now();
    let io_error = Mutex::new(None);

    thread::scope(|scope| {
        for id in 0..config.threads {
            let (file, stats, finished, io_error) = (&file, &stats, &finished, &io_error);
            scope.spawn(move || {
                let mut rng = Rng::new(config.seed ^ (id as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
                let mut tt = TranspositionTable::new(TT_MB);
                while stats.positions.load(Ordering::Relaxed) < config.positions
                    && io_error.lock().is_none()
                {
                    let Some((records, result)) = play_game(&mut rng, &mut tt, config.nodes) else { continue };
                    let bytes: Vec<u8> = records.iter().flatten().copied().collect();
                    if let Err(e) = file.lock().write_all(&bytes) {
                        io_error.lock().get_or_insert(e);
                        break;
                    }
                    stats.games.fetch_add(1, Ordering::Relaxed);
                    stats.positions.fetch_add(records.len() as u64, Ordering::Relaxed);
                    stats.results[usize::from(result)].fetch_add(1, Ordering::Relaxed);
                }
                finished.fetch_add(1, Ordering::Relaxed);
            });
        }

        let mut next_report = PROGRESS_INTERVAL;
        while finished.load(Ordering::Relaxed) < config.threads {
            thread::sleep(Duration::from_millis(100));
            if start.elapsed() >= next_report {
                report(&stats, start);
                next_report += PROGRESS_INTERVAL;
            }
        }
    });

    if let Some(e) = io_error.into_inner() {
        return Err(e);
    }
    report(&stats, start);
    Ok(())
}

fn report(stats: &Stats, start: Instant) {
    let positions = stats.positions.load(Ordering::Relaxed);
    let games = stats.games.load(Ordering::Relaxed);
    let [black, draw, white] = stats.results.each_ref().map(|r| r.load(Ordering::Relaxed));
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{positions} positions, {games} games (+{white} ={draw} -{black} for white), {:.0} pos/s, {:.0} s",
        positions as f64 / secs.max(1e-9),
        secs
    );
}

/// Plays one game; `None` if the random opening was unusable.
fn play_game(rng: &mut Rng, tt: &mut TranspositionTable, nodes: u64) -> Option<(Vec<[u8; 32]>, u8)> {
    let mut board = Board::startpos();
    let mut history = Vec::with_capacity(MAX_GAME_PLIES as usize + 16);
    for _ in 0..RANDOM_PLIES + (rng.next() & 1) as u32 {
        let mut moves = MoveList::new();
        board.generate_moves(&mut moves);
        if moves.is_empty() {
            return None;
        }
        let mv = moves[(rng.next() % moves.len() as u64) as usize];
        history.push(board.hash());
        board.make_move(mv);
    }

    tt.clear();
    let stop = AtomicBool::new(false);
    let limits = Limits { nodes: Some(nodes), ..Limits::default() };
    if game_over(&board, &history).is_some() {
        return None;
    }
    let opening = search::search(&board, history.clone(), &limits, &stop, tt, false);
    if opening.score.abs() > MAX_OPENING_SCORE {
        return None;
    }

    let mut positions: Vec<(Board, i16)> = Vec::with_capacity(MAX_GAME_PLIES as usize);
    let mut win_streak = 0i32;
    let mut draw_streak = 0u32;
    let mut ply = 0;
    let result = loop {
        if let Some(result) = game_over(&board, &history) {
            break result;
        }
        if ply >= MAX_GAME_PLIES {
            break DRAW;
        }
        let found = search::search(&board, history.clone(), &limits, &stop, tt, false);
        let white_score = if board.side_to_move() == Color::White { found.score } else { -found.score };

        win_streak = match white_score {
            s if s >= WIN_SCORE => win_streak.max(0) + 1,
            s if s <= -WIN_SCORE => win_streak.min(0) - 1,
            _ => 0,
        };
        if win_streak.abs() >= WIN_PLIES {
            break if win_streak > 0 { WHITE_WINS } else { BLACK_WINS };
        }
        draw_streak = if ply >= DRAW_MIN_PLY && white_score.abs() <= DRAW_SCORE { draw_streak + 1 } else { 0 };
        if draw_streak >= DRAW_PLIES {
            break DRAW;
        }

        // Quiet positions only: the net learns static evaluation, not tactics.
        let quiet = !board.in_check() && !found.best.is_capture() && !found.best.is_promotion();
        if quiet && !search::is_mate_score(found.score) {
            positions.push((board, found.score as i16));
        }
        history.push(board.hash());
        board.make_move(found.best);
        ply += 1;
    };

    let records = positions.iter().map(|(board, score)| encode(board, *score, result)).collect();
    Some((records, result))
}

/// White-relative result if the game has ended by rule.
fn game_over(board: &Board, history: &[u64]) -> Option<u8> {
    let mut moves = MoveList::new();
    board.generate_moves(&mut moves);
    if moves.is_empty() {
        return Some(match (board.in_check(), board.side_to_move()) {
            (false, _) => DRAW,
            (true, Color::White) => BLACK_WINS,
            (true, Color::Black) => WHITE_WINS,
        });
    }
    let draw = board.halfmove_clock() >= 100 || insufficient_material(board) || is_threefold(board, history);
    draw.then_some(DRAW)
}

/// The current position occurred twice before with the same side to move.
fn is_threefold(board: &Board, history: &[u64]) -> bool {
    let window = (board.halfmove_clock() as usize).min(history.len());
    let repeats = (2..=window).step_by(2).filter(|&back| history[history.len() - back] == board.hash()).count();
    repeats >= 2
}

/// K vs K, K+minor vs K.
fn insufficient_material(board: &Board) -> bool {
    let heavy = board.pieces(PieceType::Pawn) | board.pieces(PieceType::Rook) | board.pieces(PieceType::Queen);
    let minors = board.pieces(PieceType::Knight) | board.pieces(PieceType::Bishop);
    heavy.is_empty() && minors.count() <= 1
}

/// bullet `ChessBoard`: side-to-move relative (board mirrored vertically when
/// black moves), pieces as `(theirs << 3) | type` nibbles in square order,
/// little-endian fields: occ u64, pcs [u8; 16], score i16, result u8,
/// ksq u8, opp_ksq u8 (opponent frame), 3 padding bytes.
fn encode(board: &Board, score: i16, white_result: u8) -> [u8; 32] {
    let stm = board.side_to_move();
    let flip = if stm == Color::Black { 56 } else { 0 };
    let mut occ = 0u64;
    let mut pcs = [0u8; 16];
    let mut count = 0;
    for frame_sq in 0..64u8 {
        let Some(piece) = board.piece_at(Square::new(frame_sq ^ flip)) else { continue };
        let nibble = u8::from(piece.color() != stm) << 3 | piece.kind() as u8;
        pcs[count / 2] |= nibble << (4 * (count & 1));
        occ |= 1 << frame_sq;
        count += 1;
    }
    let result = if stm == Color::White { white_result } else { 2 - white_result };

    let mut record = [0u8; 32];
    record[0..8].copy_from_slice(&occ.to_le_bytes());
    record[8..24].copy_from_slice(&pcs);
    record[24..26].copy_from_slice(&score.to_le_bytes());
    record[26] = result;
    record[27] = board.king_square(stm).index() as u8 ^ flip;
    record[28] = board.king_square(!stm).index() as u8 ^ flip ^ 56;
    record
}

/// xorshift64*: fast, seedable, good enough for opening randomization.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[cfg(test)]
mod tests {
    use bulletformat::ChessBoard;

    use super::*;

    /// Our encoder must produce exactly what bullet's own text parser builds,
    /// for both sides to move, all results, and positive/negative scores.
    #[test]
    fn encoding_matches_bulletformat() {
        let mut rng = Rng::new(7);
        let mut checked = 0;
        for game in 0..40 {
            let mut board = Board::startpos();
            for ply in 0..120 {
                let mut moves = MoveList::new();
                board.generate_moves(&mut moves);
                if moves.is_empty() {
                    break;
                }
                let score = (rng.next() % 2001) as i16 - 1000;
                let result = ((game + ply) % 3) as u8;
                let white_score = if board.side_to_move() == Color::White { score } else { -score };
                let text = format!("{} | {white_score} | {}", board.to_fen(), ["0.0", "0.5", "1.0"][usize::from(result)]);
                let expected: ChessBoard = text.parse().unwrap();

                let ours = encode(&board, score, result);
                let got = ChessBoard {
                    occ: u64::from_le_bytes(ours[0..8].try_into().unwrap()),
                    pcs: ours[8..24].try_into().unwrap(),
                    score: i16::from_le_bytes(ours[24..26].try_into().unwrap()),
                    result: ours[26],
                    ksq: ours[27],
                    opp_ksq: ours[28],
                    extra: ours[29..32].try_into().unwrap(),
                };
                assert_eq!(got, expected, "{text}");
                checked += 1;

                board.make_move(moves[(rng.next() % moves.len() as u64) as usize]);
            }
        }
        assert!(checked > 2_000);
    }

    #[test]
    fn detects_rule_draws() {
        assert!(insufficient_material(&Board::from_fen("8/8/4k3/8/8/3NK3/8/8 w - - 0 1").unwrap()));
        assert!(!insufficient_material(&Board::from_fen("8/8/4k3/8/8/2BNK3/8/8 w - - 0 1").unwrap()));

        let mut board = Board::startpos();
        let mut history = Vec::new();
        for m in ["g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1", "f6g8"] {
            assert!(!is_threefold(&board, &history), "early repetition claim before {m}");
            history.push(board.hash());
            board.make_move(board.find_uci_move(m).unwrap());
        }
        assert!(is_threefold(&board, &history));
    }
}
