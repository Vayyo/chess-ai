use crate::{
    board::Board,
    moves::{Move, MoveList},
};

/// Number of leaf nodes of the legal move tree of `depth` plies.
pub fn perft(board: &Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut moves = MoveList::new();
    board.generate_moves(&mut moves);
    if depth == 1 {
        return moves.len() as u64;
    }
    moves
        .iter()
        .map(|&mv| {
            let mut child = *board;
            child.make_move(mv);
            perft(&child, depth - 1)
        })
        .sum()
}

/// Per-root-move leaf counts, for locating move generation bugs against a
/// reference engine. Empty for `depth == 0`.
pub fn perft_divide(board: &Board, depth: u32) -> Vec<(Move, u64)> {
    if depth == 0 {
        return Vec::new();
    }
    let mut moves = MoveList::new();
    board.generate_moves(&mut moves);
    moves
        .iter()
        .map(|&mv| {
            let mut child = *board;
            child.make_move(mv);
            (mv, perft(&child, depth - 1))
        })
        .collect()
}
