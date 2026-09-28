use std::{fmt, ops::Not};

use crate::bitboard::Bitboard;

/// Piece owner / side to move.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Color {
    White,
    Black,
}

impl Color {
    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl Not for Color {
    type Output = Color;

    #[inline(always)]
    fn not(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub enum PieceType {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl PieceType {
    pub const ALL: [PieceType; 6] = [
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ];

    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Colored piece; index = color * 6 + piece type. `Option<Piece>` is one byte.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Piece {
    WhitePawn,
    WhiteKnight,
    WhiteBishop,
    WhiteRook,
    WhiteQueen,
    WhiteKing,
    BlackPawn,
    BlackKnight,
    BlackBishop,
    BlackRook,
    BlackQueen,
    BlackKing,
}

impl Piece {
    pub const ALL: [Piece; 12] = [
        Piece::WhitePawn,
        Piece::WhiteKnight,
        Piece::WhiteBishop,
        Piece::WhiteRook,
        Piece::WhiteQueen,
        Piece::WhiteKing,
        Piece::BlackPawn,
        Piece::BlackKnight,
        Piece::BlackBishop,
        Piece::BlackRook,
        Piece::BlackQueen,
        Piece::BlackKing,
    ];

    #[inline(always)]
    pub const fn new(color: Color, kind: PieceType) -> Piece {
        Piece::ALL[color as usize * 6 + kind as usize]
    }

    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline(always)]
    pub const fn color(self) -> Color {
        if (self as u8) < 6 { Color::White } else { Color::Black }
    }

    #[inline(always)]
    pub const fn kind(self) -> PieceType {
        PieceType::ALL[self as usize % 6]
    }

    /// FEN letter: uppercase white, lowercase black.
    pub const fn to_char(self) -> char {
        b"PNBRQKpnbrqk"[self as usize] as char
    }

    pub fn from_char(c: char) -> Option<Piece> {
        "PNBRQKpnbrqk".find(c).map(|i| Piece::ALL[i])
    }
}

/// Board square: a1 = 0, b1 = 1, …, h8 = 63.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Square(pub(crate) u8);

impl Square {
    pub const A1: Square = Square(0);
    pub const B1: Square = Square(1);
    pub const C1: Square = Square(2);
    pub const D1: Square = Square(3);
    pub const E1: Square = Square(4);
    pub const F1: Square = Square(5);
    pub const G1: Square = Square(6);
    pub const H1: Square = Square(7);
    pub const A8: Square = Square(56);
    pub const B8: Square = Square(57);
    pub const C8: Square = Square(58);
    pub const D8: Square = Square(59);
    pub const E8: Square = Square(60);
    pub const F8: Square = Square(61);
    pub const G8: Square = Square(62);
    pub const H8: Square = Square(63);

    #[inline(always)]
    pub const fn new(index: u8) -> Square {
        debug_assert!(index < 64);
        Square(index)
    }

    #[inline(always)]
    pub const fn from_coords(file: u8, rank: u8) -> Square {
        Square::new(rank * 8 + file)
    }

    /// Array index. The mask proves `< 64`, removing bounds checks on `[_; 64]` tables.
    #[inline(always)]
    pub const fn index(self) -> usize {
        (self.0 & 63) as usize
    }

    #[inline(always)]
    pub const fn file(self) -> u8 {
        self.0 & 7
    }

    #[inline(always)]
    pub const fn rank(self) -> u8 {
        self.0 >> 3
    }

    #[inline(always)]
    pub const fn bb(self) -> Bitboard {
        Bitboard(1u64 << self.0)
    }

    /// Same file, mirrored rank (a1 ↔ a8).
    #[inline(always)]
    pub const fn flip_rank(self) -> Square {
        Square(self.0 ^ 56)
    }

    #[inline(always)]
    pub(crate) const fn offset(self, delta: i8) -> Square {
        Square::new(self.0.wrapping_add_signed(delta))
    }

    /// Parses algebraic coordinates such as `e4`.
    pub fn parse(s: &str) -> Option<Square> {
        let &[f, r] = s.as_bytes() else { return None };
        let (file, rank) = (f.wrapping_sub(b'a'), r.wrapping_sub(b'1'));
        (file < 8 && rank < 8).then(|| Square::from_coords(file, rank))
    }
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", (b'a' + self.file()) as char, (b'1' + self.rank()) as char)
    }
}

impl fmt::Debug for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
