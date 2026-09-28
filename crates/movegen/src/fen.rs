use std::{error::Error, fmt, fmt::Write as _};

use crate::{
    attacks::pawn_attacks,
    bitboard::Bitboard,
    board::{Board, CastleRights},
    types::{Color, Piece, PieceType, Square},
    zobrist::KEYS,
};

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FenError(String);

impl fmt::Display for FenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid FEN: {}", self.0)
    }
}

impl Error for FenError {}

fn err<T>(msg: impl Into<String>) -> Result<T, FenError> {
    Err(FenError(msg.into()))
}

impl Board {
    pub fn startpos() -> Board {
        Board::from_fen(START_FEN).expect("start position FEN is valid")
    }

    /// Parses a FEN. Castling, en passant and move counters may be omitted
    /// (defaults `-`, `-`, `0`, `1`).
    ///
    /// Rejects positions the move generator cannot handle: missing or extra
    /// kings, pawns on back ranks, material unreachable in a real game, and
    /// the side not to move being in check. Castling rights without the king
    /// and rook on their home squares are dropped; an en passant square is
    /// kept only if a capture there is pseudo-legal.
    pub fn from_fen(fen: &str) -> Result<Board, FenError> {
        let mut fields = fen.split_whitespace();
        let Some(placement) = fields.next() else { return err("empty string") };
        let Some(side) = fields.next() else { return err("missing side to move") };
        let castling = fields.next().unwrap_or("-");
        let ep = fields.next().unwrap_or("-");
        let halfmove = match fields.next() {
            Some(s) => s.parse::<u16>().or_else(|_| err(format!("bad halfmove clock `{s}`")))?,
            None => 0,
        };
        let fullmove = match fields.next() {
            Some(s) => s.parse::<u16>().or_else(|_| err(format!("bad fullmove number `{s}`")))?,
            None => 1,
        };
        if fields.next().is_some() {
            return err("trailing fields");
        }

        let mut board = Board::empty();
        let mut rank_count = 0;
        for (i, rank_str) in placement.split('/').enumerate() {
            if i >= 8 {
                return err("more than 8 ranks");
            }
            rank_count += 1;
            let rank = 7 - i as u8;
            let mut file = 0u8;
            for c in rank_str.chars() {
                if let Some(d) = c.to_digit(10) {
                    if !(1..=8).contains(&d) {
                        return err(format!("bad empty-square count `{c}`"));
                    }
                    file += d as u8;
                } else {
                    let Some(piece) = Piece::from_char(c) else { return err(format!("bad piece `{c}`")) };
                    if file >= 8 {
                        return err(format!("rank {} too long", rank + 1));
                    }
                    board.put_piece(piece, Square::from_coords(file, rank));
                    file += 1;
                }
                if file > 8 {
                    return err(format!("rank {} too long", rank + 1));
                }
            }
            if file != 8 {
                return err(format!("rank {} has {file} squares", rank + 1));
            }
        }
        if rank_count != 8 {
            return err(format!("{rank_count} ranks"));
        }
        validate_material(&board)?;

        board.side = match side {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return err(format!("bad side to move `{side}`")),
        };
        board.castling = parse_castling(&board, castling)?;
        board.ep = parse_en_passant(&board, ep)?;
        board.halfmove = halfmove;
        board.fullmove = fullmove.max(1);

        board.hash ^= KEYS.castling[board.castling.index()];
        if let Some(sq) = board.ep {
            board.hash ^= KEYS.ep_file[sq.file() as usize];
        }
        if board.side == Color::Black {
            board.hash ^= KEYS.side;
        }
        board.update_checks_and_pins();

        let them = !board.side;
        if board.is_attacked_by(board.king_square(them), board.side, board.occupied()) {
            return err("side not to move is in check");
        }
        Ok(board)
    }

    pub fn to_fen(&self) -> String {
        let mut fen = String::with_capacity(90);
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                match self.mailbox[Square::from_coords(file, rank).index()] {
                    Some(piece) => {
                        if empty > 0 {
                            fen.push(char::from(b'0' + empty));
                            empty = 0;
                        }
                        fen.push(piece.to_char());
                    }
                    None => empty += 1,
                }
            }
            if empty > 0 {
                fen.push(char::from(b'0' + empty));
            }
            if rank > 0 {
                fen.push('/');
            }
        }
        let side = if self.side == Color::White { 'w' } else { 'b' };
        write!(fen, " {side} {} ", self.castling).unwrap();
        match self.ep {
            Some(sq) => write!(fen, "{sq}").unwrap(),
            None => fen.push('-'),
        }
        write!(fen, " {} {}", self.halfmove, self.fullmove).unwrap();
        fen
    }
}

/// Bounds material to what promotions can produce; this caps legal moves at
/// 218, which `MoveList` relies on.
fn validate_material(board: &Board) -> Result<(), FenError> {
    for color in [Color::White, Color::Black] {
        let count = |kind| board.colored(color, kind).count() as i32;
        if count(PieceType::King) != 1 {
            return err(format!("{color:?} must have exactly one king"));
        }
        let pawns = count(PieceType::Pawn);
        let promoted = (count(PieceType::Queen) - 1).max(0)
            + (count(PieceType::Rook) - 2).max(0)
            + (count(PieceType::Bishop) - 2).max(0)
            + (count(PieceType::Knight) - 2).max(0);
        if pawns + promoted > 8 {
            return err(format!("{color:?} material is unreachable"));
        }
    }
    if (board.pieces(PieceType::Pawn) & (Bitboard::rank(0) | Bitboard::rank(7))).any() {
        return err("pawn on first or last rank");
    }
    Ok(())
}

fn parse_castling(board: &Board, field: &str) -> Result<CastleRights, FenError> {
    if field == "-" {
        return Ok(CastleRights::NONE);
    }
    let mut rights = CastleRights::NONE;
    for c in field.chars() {
        let (right, king_sq, rook_sq, color) = match c {
            'K' => (CastleRights::WHITE_KINGSIDE, Square::E1, Square::H1, Color::White),
            'Q' => (CastleRights::WHITE_QUEENSIDE, Square::E1, Square::A1, Color::White),
            'k' => (CastleRights::BLACK_KINGSIDE, Square::E8, Square::H8, Color::Black),
            'q' => (CastleRights::BLACK_QUEENSIDE, Square::E8, Square::A8, Color::Black),
            _ => return err(format!("bad castling field `{field}`")),
        };
        let in_place = board.piece_at(king_sq) == Some(Piece::new(color, PieceType::King))
            && board.piece_at(rook_sq) == Some(Piece::new(color, PieceType::Rook));
        if in_place {
            rights = rights.with(right);
        }
    }
    Ok(rights)
}

fn parse_en_passant(board: &Board, field: &str) -> Result<Option<Square>, FenError> {
    if field == "-" {
        return Ok(None);
    }
    let Some(sq) = Square::parse(field) else { return err(format!("bad en passant square `{field}`")) };
    let us = board.side;
    let them = !us;
    let expected_rank = if us == Color::White { 5 } else { 2 };
    if sq.rank() != expected_rank {
        return err(format!("en passant square {sq} on wrong rank"));
    }
    // The double-pushed pawn's origin must be empty too, or the push could not have happened.
    let origin = sq.offset(if us == Color::White { 8 } else { -8 });
    let capturable = board.piece_at(sq).is_none()
        && board.piece_at(origin).is_none()
        && board.piece_at(Square::new(sq.0 ^ 8)) == Some(Piece::new(them, PieceType::Pawn))
        && (pawn_attacks(them, sq) & board.colored(us, PieceType::Pawn)).any();
    Ok(capturable.then_some(sq))
}
