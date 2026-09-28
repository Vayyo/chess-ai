//! `chess-ai bench [depth]`: fixed-depth searches over a position set.
//!
//! The node total is a search signature: changes meant to be non-functional
//! must leave it identical; functional changes record the new value in the
//! commit message. NPS measures raw speed.

use std::{sync::atomic::AtomicBool, time::Instant};

use movegen::Board;

use crate::{
    search::{self, Limits},
    tt::{self, TranspositionTable},
};

pub const DEFAULT_DEPTH: u32 = 6;

const POSITIONS: [&str; 12] = [
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
    "rnbqkb1r/pppppppp/5n2/8/2PP4/8/PP2PPPP/RNBQKBNR b KQkq - 0 2",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
    "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
    "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1",
    "8/5pk1/6p1/8/3R4/6P1/5PK1/2r5 w - - 0 1",
    "8/8/8/4k3/8/8/4P3/4K3 w - - 0 1",
    "4r1k1/1p3ppp/p1p5/3n4/3P4/1BN5/PP3PPP/4R1K1 b - - 0 20",
];

pub fn run(depth: u32) {
    let stop = AtomicBool::new(false);
    let limits = Limits { depth: Some(depth), ..Limits::default() };
    let mut tt = TranspositionTable::new(tt::DEFAULT_MB);
    let start = Instant::now();
    let mut nodes = 0;
    for fen in POSITIONS {
        let board = Board::from_fen(fen).expect("bench FEN is valid");
        // Fresh table per position keeps the signature independent of order.
        tt.clear();
        let result = search::search(&board, Vec::new(), &limits, &stop, &mut tt, false);
        println!("{:>10} {} {fen}", result.nodes, result.best);
        nodes += result.nodes;
    }
    let secs = start.elapsed().as_secs_f64();
    println!("{nodes} nodes {:.0} nps", nodes as f64 / secs.max(1e-9));
}
