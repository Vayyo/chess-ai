use std::{
    fmt,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    slice,
};

use crate::types::{PieceType, Square};

/// 16-bit move: bits 0–5 from, 6–11 to, 12–15 flag.
///
/// Flag layout: bit 3 = promotion, bit 2 = capture; for promotions the low
/// two bits select knight/bishop/rook/queen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Move(u16);

impl Move {
    /// `a1a1`; never generated. Printed as `0000` in UCI.
    pub const NULL: Move = Move(0);

    pub(crate) const QUIET: u16 = 0;
    pub(crate) const DOUBLE_PUSH: u16 = 1;
    pub(crate) const KING_CASTLE: u16 = 2;
    pub(crate) const QUEEN_CASTLE: u16 = 3;
    pub(crate) const CAPTURE: u16 = 4;
    pub(crate) const EN_PASSANT: u16 = 5;
    pub(crate) const PROMOTION: u16 = 8;
    pub(crate) const PROMOTION_CAPTURE: u16 = 12;

    #[inline(always)]
    pub(crate) const fn new(from: Square, to: Square, flag: u16) -> Move {
        Move(from.0 as u16 | (to.0 as u16) << 6 | flag << 12)
    }

    /// Inverse of [`Move::raw`]. The result is only meaningful in a position
    /// that generates it; compare against a generated `MoveList` before playing.
    #[inline(always)]
    pub const fn from_raw(raw: u16) -> Move {
        Move(raw)
    }

    #[inline(always)]
    pub const fn from(self) -> Square {
        Square::new((self.0 & 63) as u8)
    }

    #[inline(always)]
    pub const fn to(self) -> Square {
        Square::new((self.0 >> 6 & 63) as u8)
    }

    #[inline(always)]
    pub(crate) const fn flag(self) -> u16 {
        self.0 >> 12
    }

    /// Raw 16-bit encoding (for hash tables and training data).
    #[inline(always)]
    pub const fn raw(self) -> u16 {
        self.0
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    /// Includes en passant and capturing promotions.
    #[inline(always)]
    pub const fn is_capture(self) -> bool {
        self.flag() & Self::CAPTURE != 0
    }

    #[inline(always)]
    pub const fn is_en_passant(self) -> bool {
        self.flag() == Self::EN_PASSANT
    }

    #[inline(always)]
    pub const fn is_castle(self) -> bool {
        matches!(self.flag(), Self::KING_CASTLE | Self::QUEEN_CASTLE)
    }

    #[inline(always)]
    pub const fn is_promotion(self) -> bool {
        self.flag() & Self::PROMOTION != 0
    }

    #[inline(always)]
    pub const fn promotion(self) -> Option<PieceType> {
        if self.is_promotion() {
            Some(PieceType::ALL[1 + (self.flag() & 3) as usize])
        } else {
            None
        }
    }
}

/// UCI long algebraic notation: `e2e4`, `e7e8q`, castling as king move `e1g1`.
impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_null() {
            return f.write_str("0000");
        }
        write!(f, "{}{}", self.from(), self.to())?;
        match self.promotion() {
            Some(PieceType::Knight) => f.write_str("n"),
            Some(PieceType::Bishop) => f.write_str("b"),
            Some(PieceType::Rook) => f.write_str("r"),
            Some(_) => f.write_str("q"),
            None => Ok(()),
        }
    }
}

impl fmt::Debug for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Fixed-capacity move buffer; no heap, no zeroing.
///
/// FEN validation bounds material to what a real game can reach, which caps
/// legal moves at 218; the `u8` length keeps every index in bounds.
pub struct MoveList {
    moves: [MaybeUninit<Move>; 256],
    len: u8,
}

impl MoveList {
    #[inline(always)]
    pub fn new() -> MoveList {
        MoveList { moves: [MaybeUninit::uninit(); 256], len: 0 }
    }

    #[inline(always)]
    pub(crate) fn push(&mut self, mv: Move) {
        self.moves[self.len as usize] = MaybeUninit::new(mv);
        self.len += 1;
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[Move] {
        // SAFETY: elements `0..len` were written by `push` before `len` grew.
        unsafe { slice::from_raw_parts(self.moves.as_ptr().cast(), self.len as usize) }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        // SAFETY: as in `as_slice`.
        unsafe { slice::from_raw_parts_mut(self.moves.as_mut_ptr().cast(), self.len as usize) }
    }
}

impl Default for MoveList {
    fn default() -> MoveList {
        MoveList::new()
    }
}

impl Deref for MoveList {
    type Target = [Move];

    #[inline(always)]
    fn deref(&self) -> &[Move] {
        self.as_slice()
    }
}

impl DerefMut for MoveList {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [Move] {
        self.as_mut_slice()
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = slice::Iter<'a, Move>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl fmt::Debug for MoveList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}
