//! UCI protocol. Searches run on a worker thread so `stop`/`isready` stay responsive.
//! Extensions: `go perft <depth>` (divide output) and `d` (print the position).

use std::{
    io::{self, BufRead},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use movegen::{Board, perft_divide};

use crate::{
    search::{self, Limits},
    tt::{self, TranspositionTable},
};

const NAME: &str = concat!("chess-ai ", env!("CARGO_PKG_VERSION"));
/// Recursion depth is bounded by MAX_PLY, each frame holds a MoveList and a Board.
const SEARCH_STACK_BYTES: usize = 16 << 20;

pub fn run() {
    let mut uci = Uci {
        board: Board::startpos(),
        history: Vec::new(),
        stop: Arc::default(),
        tt: Some(TranspositionTable::new(tt::DEFAULT_MB)),
        worker: None,
    };
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if !uci.handle(&line) {
            break;
        }
    }
    uci.stop_search();
}

struct Uci {
    board: Board,
    /// Hashes of positions before `board` in the current game, oldest first.
    history: Vec<u64>,
    stop: Arc<AtomicBool>,
    /// The table moves into the search thread and comes back through `worker`;
    /// exactly one of `tt` and `worker` is `Some`.
    tt: Option<TranspositionTable>,
    worker: Option<JoinHandle<TranspositionTable>>,
}

impl Uci {
    /// Returns `false` on `quit`.
    fn handle(&mut self, line: &str) -> bool {
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("uci") => {
                println!("id name {NAME}\nid author chess-ai contributors");
                println!("option name Hash type spin default {} min 1 max {}", tt::DEFAULT_MB, tt::MAX_MB);
                println!("uciok");
            }
            Some("isready") => println!("readyok"),
            Some("setoption") => self.set_option(tokens),
            Some("ucinewgame") => {
                self.table().clear();
                self.board = Board::startpos();
                self.history.clear();
            }
            Some("position") => {
                self.stop_search();
                self.position(tokens);
            }
            Some("go") => {
                self.stop_search();
                self.go(tokens);
            }
            Some("stop") => self.stop_search(),
            Some("quit") => return false,
            Some("d") => println!("{:?}", self.board),
            _ => {}
        }
        true
    }

    fn position<'a>(&mut self, mut tokens: impl Iterator<Item = &'a str>) {
        let board = match tokens.next() {
            Some("startpos") => Board::startpos(),
            Some("fen") => {
                let fen: Vec<&str> = tokens.by_ref().take_while(|&t| t != "moves").collect();
                match Board::from_fen(&fen.join(" ")) {
                    Ok(board) => board,
                    Err(e) => {
                        println!("info string {e}");
                        return;
                    }
                }
            }
            _ => {
                println!("info string expected `position startpos` or `position fen <fen>`");
                return;
            }
        };
        self.board = board;
        self.history.clear();
        for token in tokens.filter(|&t| t != "moves") {
            let Some(mv) = self.board.find_uci_move(token) else {
                println!("info string illegal move {token}, ignoring the rest");
                return;
            };
            self.history.push(self.board.hash());
            self.board.make_move(mv);
        }
    }

    fn go<'a>(&mut self, mut tokens: impl Iterator<Item = &'a str>) {
        let mut limits = Limits::default();
        while let Some(token) = tokens.next() {
            match token {
                "perft" => {
                    self.perft(value(&mut tokens).unwrap_or(1));
                    return;
                }
                "depth" => limits.depth = value(&mut tokens),
                "nodes" => limits.nodes = value(&mut tokens),
                "movetime" => limits.movetime = millis(&mut tokens),
                "wtime" => limits.time[0] = millis(&mut tokens),
                "btime" => limits.time[1] = millis(&mut tokens),
                "winc" => limits.inc[0] = millis(&mut tokens).unwrap_or(0),
                "binc" => limits.inc[1] = millis(&mut tokens).unwrap_or(0),
                "movestogo" => limits.movestogo = value(&mut tokens),
                "infinite" => limits.infinite = true,
                _ => {}
            }
        }

        self.stop.store(false, Ordering::Relaxed);
        let board = self.board;
        let history = self.history.clone();
        let stop = Arc::clone(&self.stop);
        let mut tt = self.tt.take().expect("no search is running, so the table is here");
        let worker = thread::Builder::new()
            .name("search".into())
            .stack_size(SEARCH_STACK_BYTES)
            .spawn(move || {
                search::search(&board, history, &limits, &stop, &mut tt, true);
                tt
            })
            .expect("spawn search thread");
        self.worker = Some(worker);
    }

    fn stop_search(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.stop.store(true, Ordering::Relaxed);
            self.tt = Some(worker.join().expect("search thread panicked"));
        }
    }

    /// The transposition table, waiting for a running search to hand it back.
    fn table(&mut self) -> &mut TranspositionTable {
        self.stop_search();
        self.tt.as_mut().expect("table is returned by stop_search")
    }

    /// `setoption name <id> [value <x>]`
    fn set_option<'a>(&mut self, tokens: impl Iterator<Item = &'a str>) {
        let words: Vec<&str> = tokens.collect();
        let value_at = words.iter().position(|&w| w == "value");
        let name = words.get(1..value_at.unwrap_or(words.len())).unwrap_or_default().join(" ");
        let value = value_at.map(|i| words[i + 1..].join(" "));
        match name.to_ascii_lowercase().as_str() {
            "hash" => match value.and_then(|v| v.parse::<usize>().ok()) {
                Some(mb) if (1..=tt::MAX_MB).contains(&mb) => {
                    self.stop_search();
                    // Drop the old table first so peak memory is one table, not two.
                    self.tt = None;
                    self.tt = Some(TranspositionTable::new(mb));
                }
                _ => println!("info string Hash must be 1..={} MB", tt::MAX_MB),
            },
            _ => println!("info string unknown option `{name}`"),
        }
    }

    fn perft(&self, depth: u32) {
        let start = Instant::now();
        let mut total = 0;
        for (mv, nodes) in perft_divide(&self.board, depth.max(1)) {
            println!("{mv}: {nodes}");
            total += nodes;
        }
        let secs = start.elapsed().as_secs_f64();
        println!("\nNodes searched: {total}\nTime: {secs:.3} s ({:.1} Mnps)\n", total as f64 / secs.max(1e-9) / 1e6);
    }
}

fn value<'a, T: FromStr>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<T> {
    tokens.next()?.parse().ok()
}

/// Clock values; some GUIs send negative remaining time when flagging.
fn millis<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<u64> {
    value::<i64>(tokens).map(|ms| ms.max(0) as u64)
}
