use std::{
    fmt,
    ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not, Shl, Shr},
};

use crate::types::Square;

/// Set of squares; bit `i` is square `i` (a1 = bit 0).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Bitboard(pub u64);

impl Bitboard {
    pub const EMPTY: Bitboard = Bitboard(0);
    pub const FULL: Bitboard = Bitboard(!0);

    /// File `0..8` (a..h).
    #[inline(always)]
    pub const fn file(file: u8) -> Bitboard {
        Bitboard(0x0101_0101_0101_0101 << file)
    }

    /// Rank `0..8` (1..8).
    #[inline(always)]
    pub const fn rank(rank: u8) -> Bitboard {
        Bitboard(0xFF << (rank * 8))
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub const fn any(self) -> bool {
        self.0 != 0
    }

    #[inline(always)]
    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }

    #[inline(always)]
    pub const fn more_than_one(self) -> bool {
        self.0 & self.0.wrapping_sub(1) != 0
    }

    #[inline(always)]
    pub const fn contains(self, sq: Square) -> bool {
        self.0 & (1u64 << sq.0) != 0
    }

    /// Lowest square. The set must be non-empty.
    #[inline(always)]
    pub const fn lsb(self) -> Square {
        debug_assert!(self.0 != 0);
        Square::new(self.0.trailing_zeros() as u8)
    }

    /// Removes and returns the lowest square. The set must be non-empty.
    #[inline(always)]
    pub fn pop_lsb(&mut self) -> Square {
        let sq = self.lsb();
        self.0 &= self.0 - 1;
        sq
    }
}

macro_rules! bit_ops {
    ($($trait:ident $fn:ident $assign_trait:ident $assign_fn:ident $op:tt;)*) => {$(
        impl $trait for Bitboard {
            type Output = Bitboard;

            #[inline(always)]
            fn $fn(self, rhs: Bitboard) -> Bitboard {
                Bitboard(self.0 $op rhs.0)
            }
        }

        impl $assign_trait for Bitboard {
            #[inline(always)]
            fn $assign_fn(&mut self, rhs: Bitboard) {
                self.0 = self.0 $op rhs.0;
            }
        }
    )*};
}

bit_ops! {
    BitAnd bitand BitAndAssign bitand_assign &;
    BitOr bitor BitOrAssign bitor_assign |;
    BitXor bitxor BitXorAssign bitxor_assign ^;
}

impl Not for Bitboard {
    type Output = Bitboard;

    #[inline(always)]
    fn not(self) -> Bitboard {
        Bitboard(!self.0)
    }
}

impl Shl<u32> for Bitboard {
    type Output = Bitboard;

    #[inline(always)]
    fn shl(self, rhs: u32) -> Bitboard {
        Bitboard(self.0 << rhs)
    }
}

impl Shr<u32> for Bitboard {
    type Output = Bitboard;

    #[inline(always)]
    fn shr(self, rhs: u32) -> Bitboard {
        Bitboard(self.0 >> rhs)
    }
}

/// Iterates squares from lowest to highest.
impl Iterator for Bitboard {
    type Item = Square;

    #[inline(always)]
    fn next(&mut self) -> Option<Square> {
        if self.0 == 0 { None } else { Some(self.pop_lsb()) }
    }

    #[inline(always)]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.count() as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for Bitboard {}

impl fmt::Debug for Bitboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Bitboard({:#018x})", self.0)?;
        for rank in (0..8).rev() {
            for file in 0..8 {
                let c = if self.contains(Square::from_coords(file, rank)) { 'x' } else { '.' };
                write!(f, " {c}")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}
