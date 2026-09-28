//! Reference node counts from https://www.chessprogramming.org/Perft_Results.
//! Debug assertions verify the incremental Zobrist hash after every move.

use movegen::{Board, MoveList, START_FEN, perft};

fn check(fen: &str, expected: &[u64]) {
    let board = Board::from_fen(fen).unwrap();
    for (depth, &nodes) in (1..).zip(expected) {
        assert_eq!(perft(&board, depth), nodes, "{fen} depth {depth}");
    }
}

#[test]
fn start_position() {
    check(START_FEN, &[20, 400, 8_902, 197_281, 4_865_609]);
}

#[test]
fn kiwipete() {
    check(
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        &[48, 2_039, 97_862, 4_085_603],
    );
}

#[test]
fn endgame_en_passant_pins() {
    check("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", &[14, 191, 2_812, 43_238, 674_624, 11_030_083]);
}

#[test]
fn promotions_and_castling() {
    check("r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1", &[6, 264, 9_467, 422_333]);
    // Color-mirrored twin must give identical counts.
    check("r2q1rk1/pP1p2pp/Q4n2/bbp1p3/Np6/1B3NBn/pPPP1PPP/R3K2R b KQ - 0 1", &[6, 264, 9_467, 422_333]);
}

#[test]
fn discovered_checks() {
    check("rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8", &[44, 1_486, 62_379, 2_103_487]);
}

#[test]
fn middlegame() {
    check(
        "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
        &[46, 2_079, 89_890, 3_894_594],
    );
}

#[test]
fn transpositions_share_hash() {
    let play = |moves: &[&str]| {
        let mut board = Board::startpos();
        for m in moves {
            let mv = board.find_uci_move(m).unwrap();
            board.make_move(mv);
        }
        board
    };
    let a = play(&["g1f3", "g8f6", "b1c3", "b8c6"]);
    let b = play(&["b1c3", "b8c6", "g1f3", "g8f6"]);
    assert_eq!(a.hash(), b.hash());
    assert_eq!(play(&["g1f3", "g8f6", "f3g1", "f6g8"]).hash(), Board::startpos().hash());
    // Same placement, different side to move.
    assert_ne!(play(&["g1f3", "g8f6", "f3g1"]).hash(), play(&["g1f3"]).hash());
}

#[test]
fn fen_round_trip() {
    for fen in [
        START_FEN,
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
    ] {
        assert_eq!(Board::from_fen(fen).unwrap().to_fen(), fen);
    }
}

#[test]
fn fen_drops_impossible_en_passant() {
    // c7 is occupied, so c7-c5 cannot have just been played: no dxc6 e.p.
    let board = Board::from_fen("4k3/2p5/8/2pP4/8/8/8/4K3 w - c6 0 1").unwrap();
    assert_eq!(board.en_passant(), None);
    assert!(board.find_uci_move("d5c6").is_none());
    // Same structure with c7 empty keeps it.
    let board = Board::from_fen("4k3/8/8/2pP4/8/8/8/4K3 w - c6 0 1").unwrap();
    assert!(board.find_uci_move("d5c6").is_some_and(|m| m.is_en_passant()));
}

#[test]
fn fen_rejects_broken_positions() {
    for fen in [
        "8/8/8/8/8/8/8/8 w - - 0 1",                                  // no kings
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR x KQkq - 0 1",   // bad side
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBN w KQkq - 0 1",    // short rank
        "4k3/8/8/8/8/8/8/4K2P w - - 0 1",                             // pawn on back rank
        "QQQQQQQQ/QQQQQQQQ/8/8/8/8/8/K6k w - - 0 1",                   // unreachable material
        "4k3/4R3/8/8/8/8/8/4K3 w - - 0 1",                            // side not to move in check
    ] {
        assert!(Board::from_fen(fen).is_err(), "{fen}");
    }
}

/// Every node of the trees: `generate_noisy` is exactly the captures and
/// promotions of `generate_moves`, and every move round-trips through UCI text.
#[test]
fn noisy_moves_and_uci_parsing_match_full_generation() {
    fn walk(board: &Board, depth: u32) {
        let mut all = MoveList::new();
        board.generate_moves(&mut all);
        let mut noisy = MoveList::new();
        board.generate_noisy(&mut noisy);

        let mut expected: Vec<u16> =
            all.iter().filter(|m| m.is_capture() || m.is_promotion()).map(|m| m.raw()).collect();
        let mut got: Vec<u16> = noisy.iter().map(|m| m.raw()).collect();
        expected.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, expected, "noisy moves differ in {}", board.to_fen());

        for &mv in &all {
            assert_eq!(board.find_uci_move(&mv.to_string()), Some(mv), "{mv} in {}", board.to_fen());
            if depth > 1 {
                let mut child = *board;
                child.make_move(mv);
                walk(&child, depth - 1);
            }
        }
    }

    for fen in [
        START_FEN,
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
    ] {
        walk(&Board::from_fen(fen).unwrap(), 3);
    }
}
