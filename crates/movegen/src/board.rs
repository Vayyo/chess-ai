use std::fmt;

use crate::{
    attacks::{between, bishop_attacks, king_attacks, knight_attacks, pawn_attacks, rook_attacks},
    bitboard::Bitboard,
    moves::Move,
    types::{Color, Piece, PieceType, Square},
    zobrist::KEYS,
};

/// Castling availability as a 4-bit set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CastleRights(u8);

impl CastleRights {
    pub const NONE: CastleRights = CastleRights(0);
    pub const WHITE_KINGSIDE: CastleRights = CastleRights(1);
    pub const WHITE_QUEENSIDE: CastleRights = CastleRights(2);
    pub const BLACK_KINGSIDE: CastleRights = CastleRights(4);
    pub const BLACK_QUEENSIDE: CastleRights = CastleRights(8);
    pub const ALL: CastleRights = CastleRights(15);

    #[inline(always)]
    pub const fn contains(self, other: CastleRights) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline(always)]
    pub(crate) const fn with(self, other: CastleRights) -> CastleRights {
        CastleRights(self.0 | other.0)
    }

    #[inline(always)]
    pub(crate) const fn without(self, other: CastleRights) -> CastleRights {
        CastleRights(self.0 & !other.0)
    }

    #[inline(always)]
    pub(crate) const fn index(self) -> usize {
        (self.0 & 15) as usize
    }

    #[inline(always)]
    pub(crate) const fn kingside(color: Color) -> CastleRights {
        match color {
            Color::White => Self::WHITE_KINGSIDE,
            Color::Black => Self::BLACK_KINGSIDE,
        }
    }

    #[inline(always)]
    pub(crate) const fn queenside(color: Color) -> CastleRights {
        match color {
            Color::White => Self::WHITE_QUEENSIDE,
            Color::Black => Self::BLACK_QUEENSIDE,
        }
    }
}

/// FEN field: `KQkq` subset or `-`.
impl fmt::Display for CastleRights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return f.write_str("-");
        }
        for (right, c) in [
            (Self::WHITE_KINGSIDE, 'K'),
            (Self::WHITE_QUEENSIDE, 'Q'),
            (Self::BLACK_KINGSIDE, 'k'),
            (Self::BLACK_QUEENSIDE, 'q'),
        ] {
            if self.contains(right) {
                write!(f, "{c}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for CastleRights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Rights surviving a move that touches each square: moving from or capturing
/// on a king/rook home square clears the matching rights.
static CASTLE_KEEP: [CastleRights; 64] = {
    let mut t = [CastleRights::ALL; 64];
    t[Square::A1.index()] = CastleRights::ALL.without(CastleRights::WHITE_QUEENSIDE);
    t[Square::H1.index()] = CastleRights::ALL.without(CastleRights::WHITE_KINGSIDE);
    t[Square::E1.index()] =
        CastleRights::ALL.without(CastleRights::WHITE_KINGSIDE).without(CastleRights::WHITE_QUEENSIDE);
    t[Square::A8.index()] = CastleRights::ALL.without(CastleRights::BLACK_QUEENSIDE);
    t[Square::H8.index()] = CastleRights::ALL.without(CastleRights::BLACK_KINGSIDE);
    t[Square::E8.index()] =
        CastleRights::ALL.without(CastleRights::BLACK_KINGSIDE).without(CastleRights::BLACK_QUEENSIDE);
    t
};

/// Chess position. `Copy` by design: search uses copy-make (copy, then
/// `make_move` on the copy) instead of unmake.
///
/// Invariants: exactly one king per side; `ep` is set only when an en passant
/// capture is pseudo-legal; `checkers`/`pinned` describe the side to move.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Board {
    pub(crate) pieces: [Bitboard; 6],
    pub(crate) colors: [Bitboard; 2],
    pub(crate) mailbox: [Option<Piece>; 64],
    pub(crate) side: Color,
    pub(crate) castling: CastleRights,
    pub(crate) ep: Option<Square>,
    pub(crate) halfmove: u16,
    pub(crate) fullmove: u16,
    pub(crate) hash: u64,
    /// Enemy pieces giving check to the side to move.
    pub(crate) checkers: Bitboard,
    /// Pieces of the side to move pinned to their own king.
    pub(crate) pinned: Bitboard,
}

impl Board {
    pub(crate) const fn empty() -> Board {
        Board {
            pieces: [Bitboard::EMPTY; 6],
            colors: [Bitboard::EMPTY; 2],
            mailbox: [None; 64],
            side: Color::White,
            castling: CastleRights::NONE,
            ep: None,
            halfmove: 0,
            fullmove: 1,
            hash: 0,
            checkers: Bitboard::EMPTY,
            pinned: Bitboard::EMPTY,
        }
    }

    #[inline(always)]
    pub fn side_to_move(&self) -> Color {
        self.side
    }

    /// Pieces of a type, both colors.
    #[inline(always)]
    pub fn pieces(&self, kind: PieceType) -> Bitboard {
        self.pieces[kind.index()]
    }

    #[inline(always)]
    pub fn color(&self, color: Color) -> Bitboard {
        self.colors[color.index()]
    }

    #[inline(always)]
    pub fn colored(&self, color: Color, kind: PieceType) -> Bitboard {
        self.pieces[kind.index()] & self.colors[color.index()]
    }

    #[inline(always)]
    pub fn occupied(&self) -> Bitboard {
        self.colors[0] | self.colors[1]
    }

    #[inline(always)]
    pub fn piece_at(&self, sq: Square) -> Option<Piece> {
        self.mailbox[sq.index()]
    }

    #[inline(always)]
    pub fn king_square(&self, color: Color) -> Square {
        self.colored(color, PieceType::King).lsb()
    }

    #[inline(always)]
    pub fn checkers(&self) -> Bitboard {
        self.checkers
    }

    #[inline(always)]
    pub fn in_check(&self) -> bool {
        self.checkers.any()
    }

    #[inline(always)]
    pub fn pinned(&self) -> Bitboard {
        self.pinned
    }

    #[inline(always)]
    pub fn castling(&self) -> CastleRights {
        self.castling
    }

    /// En passant target square, present only when a capture there is pseudo-legal.
    #[inline(always)]
    pub fn en_passant(&self) -> Option<Square> {
        self.ep
    }

    /// Plies since the last capture or pawn move.
    #[inline(always)]
    pub fn halfmove_clock(&self) -> u16 {
        self.halfmove
    }

    #[inline(always)]
    pub fn fullmove_number(&self) -> u16 {
        self.fullmove
    }

    /// Zobrist hash of placement, side, castling rights and en passant file.
    #[inline(always)]
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Pieces of both colors attacking `sq` given occupancy `occ`.
    pub fn attackers_to(&self, sq: Square, occ: Bitboard) -> Bitboard {
        let [pawns, knights, bishops, rooks, queens, kings] = self.pieces;
        (pawn_attacks(Color::White, sq) & pawns & self.colors[Color::Black.index()])
            | (pawn_attacks(Color::Black, sq) & pawns & self.colors[Color::White.index()])
            | (knight_attacks(sq) & knights)
            | (king_attacks(sq) & kings)
            | (bishop_attacks(sq, occ) & (bishops | queens))
            | (rook_attacks(sq, occ) & (rooks | queens))
    }

    /// Whether any piece of `by` attacks `sq` given occupancy `occ`.
    #[inline]
    pub fn is_attacked_by(&self, sq: Square, by: Color, occ: Bitboard) -> bool {
        let theirs = self.colors[by.index()];
        let [pawns, knights, bishops, rooks, queens, kings] = self.pieces;
        (knight_attacks(sq) & knights & theirs).any()
            || (pawn_attacks(!by, sq) & pawns & theirs).any()
            || (king_attacks(sq) & kings & theirs).any()
            || (bishop_attacks(sq, occ) & (bishops | queens) & theirs).any()
            || (rook_attacks(sq, occ) & (rooks | queens) & theirs).any()
    }

    /// Plays a legal move produced by this position's move generator.
    ///
    /// # Panics
    /// If `mv` moves from an empty square or captures on an empty square.
    /// Other illegal moves corrupt the position.
    pub fn make_move(&mut self, mv: Move) {
        let us = self.side;
        let them = !us;
        let (from, to) = (mv.from(), mv.to());
        let Some(piece) = self.mailbox[from.index()] else {
            panic!("make_move {mv}: no piece on {from}");
        };

        self.halfmove = self.halfmove.saturating_add(1);
        if let Some(ep) = self.ep.take() {
            self.hash ^= KEYS.ep_file[ep.file() as usize];
        }

        match mv.flag() {
            Move::EN_PASSANT => {
                self.remove_piece(Piece::new(them, PieceType::Pawn), Square::new(to.0 ^ 8));
                self.move_piece(piece, from, to);
            }
            Move::KING_CASTLE => {
                self.move_piece(piece, from, to);
                self.move_piece(Piece::new(us, PieceType::Rook), to.offset(1), to.offset(-1));
            }
            Move::QUEEN_CASTLE => {
                self.move_piece(piece, from, to);
                self.move_piece(Piece::new(us, PieceType::Rook), to.offset(-2), to.offset(1));
            }
            _ => {
                if mv.is_capture() {
                    let Some(captured) = self.mailbox[to.index()] else {
                        panic!("make_move {mv}: nothing to capture on {to}");
                    };
                    self.remove_piece(captured, to);
                    self.halfmove = 0;
                }
                if let Some(promo) = mv.promotion() {
                    self.remove_piece(piece, from);
                    self.put_piece(Piece::new(us, promo), to);
                } else {
                    self.move_piece(piece, from, to);
                }
                if mv.flag() == Move::DOUBLE_PUSH {
                    let ep = Square::new((from.0 + to.0) / 2);
                    if (pawn_attacks(us, ep) & self.colored(them, PieceType::Pawn)).any() {
                        self.ep = Some(ep);
                        self.hash ^= KEYS.ep_file[ep.file() as usize];
                    }
                }
            }
        }
        if piece.kind() == PieceType::Pawn {
            self.halfmove = 0;
        }

        let rights = CastleRights(self.castling.0 & CASTLE_KEEP[from.index()].0 & CASTLE_KEEP[to.index()].0);
        if rights != self.castling {
            self.hash ^= KEYS.castling[self.castling.index()] ^ KEYS.castling[rights.index()];
            self.castling = rights;
        }

        self.side = them;
        self.hash ^= KEYS.side;
        if us == Color::Black {
            self.fullmove = self.fullmove.saturating_add(1);
        }
        self.update_checks_and_pins();
        debug_assert_eq!(self.hash, self.compute_hash(), "incremental hash diverged after {mv}");
    }

    /// Passes the turn (null-move pruning). The side to move must not be in check.
    pub fn make_null_move(&mut self) {
        debug_assert!(!self.in_check(), "null move while in check");
        if let Some(ep) = self.ep.take() {
            self.hash ^= KEYS.ep_file[ep.file() as usize];
        }
        self.halfmove = self.halfmove.saturating_add(1);
        if self.side == Color::Black {
            self.fullmove = self.fullmove.saturating_add(1);
        }
        self.side = !self.side;
        self.hash ^= KEYS.side;
        self.update_checks_and_pins();
    }

    #[inline(always)]
    pub(crate) fn put_piece(&mut self, piece: Piece, sq: Square) {
        let bb = sq.bb();
        self.pieces[piece.kind().index()] |= bb;
        self.colors[piece.color().index()] |= bb;
        self.mailbox[sq.index()] = Some(piece);
        self.hash ^= KEYS.pieces[piece.index()][sq.index()];
    }

    #[inline(always)]
    fn remove_piece(&mut self, piece: Piece, sq: Square) {
        let bb = sq.bb();
        self.pieces[piece.kind().index()] ^= bb;
        self.colors[piece.color().index()] ^= bb;
        self.mailbox[sq.index()] = None;
        self.hash ^= KEYS.pieces[piece.index()][sq.index()];
    }

    #[inline(always)]
    fn move_piece(&mut self, piece: Piece, from: Square, to: Square) {
        let bb = from.bb() | to.bb();
        self.pieces[piece.kind().index()] ^= bb;
        self.colors[piece.color().index()] ^= bb;
        self.mailbox[from.index()] = None;
        self.mailbox[to.index()] = Some(piece);
        let keys = &KEYS.pieces[piece.index()];
        self.hash ^= keys[from.index()] ^ keys[to.index()];
    }

    pub(crate) fn update_checks_and_pins(&mut self) {
        let us = self.side;
        let ksq = self.king_square(us);
        let occ = self.occupied();
        let theirs = self.colors[(!us).index()];
        let [pawns, knights, bishops, rooks, queens, _] = self.pieces;
        let diagonal = (bishops | queens) & theirs;
        let orthogonal = (rooks | queens) & theirs;

        self.checkers = ((pawn_attacks(us, ksq) & pawns) | (knight_attacks(ksq) & knights)) & theirs
            | (bishop_attacks(ksq, occ) & diagonal)
            | (rook_attacks(ksq, occ) & orthogonal);

        let snipers =
            (bishop_attacks(ksq, Bitboard::EMPTY) & diagonal) | (rook_attacks(ksq, Bitboard::EMPTY) & orthogonal);
        let mut pinned = Bitboard::EMPTY;
        for sniper in snipers {
            let blockers = between(ksq, sniper) & occ;
            if blockers.any() && !blockers.more_than_one() {
                pinned |= blockers;
            }
        }
        self.pinned = pinned & self.colors[us.index()];
    }

    /// Full recomputation of `hash`; reference for the incremental updates.
    pub(crate) fn compute_hash(&self) -> u64 {
        let mut hash = 0;
        for (i, piece) in self.mailbox.iter().enumerate() {
            if let Some(p) = piece {
                hash ^= KEYS.pieces[p.index()][i];
            }
        }
        hash ^= KEYS.castling[self.castling.index()];
        if let Some(ep) = self.ep {
            hash ^= KEYS.ep_file[ep.file() as usize];
        }
        if self.side == Color::Black {
            hash ^= KEYS.side;
        }
        hash
    }
}

impl fmt::Debug for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for rank in (0..8).rev() {
            write!(f, "{} ", rank + 1)?;
            for file in 0..8 {
                let c = self.mailbox[Square::from_coords(file, rank).index()].map_or('.', Piece::to_char);
                write!(f, " {c}")?;
            }
            writeln!(f)?;
        }
        writeln!(f, "   a b c d e f g h")?;
        writeln!(f, "FEN: {}", self.to_fen())?;
        write!(f, "Hash: {:016x}", self.hash)
    }
}
