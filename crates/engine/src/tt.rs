//! Transposition table: one 16-byte entry per slot, indexed by the full
//! Zobrist key, depth-preferred replacement with per-search aging.

use movegen::Move;

pub const DEFAULT_MB: usize = 16;
pub const MAX_MB: usize = 8192;

/// What the stored score proves about the true value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Bound {
    /// Fail low: true score <= stored.
    Upper = 1,
    /// Fail high: true score >= stored.
    Lower = 2,
    Exact = 3,
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub mv: Move,
    /// Ply-relative mate scores are converted back by the caller (see `search`).
    pub score: i32,
    pub depth: i32,
    pub bound: Bound,
}

#[derive(Clone, Copy, Default)]
#[repr(C, align(16))]
struct Entry {
    key: u64,
    mv: u16,
    score: i16,
    depth: u8,
    /// `age << 2 | bound`; bound 0 marks an empty slot.
    meta: u8,
}

pub struct TranspositionTable {
    entries: Vec<Entry>,
    age: u8,
}

impl TranspositionTable {
    pub fn new(mb: usize) -> TranspositionTable {
        let len = (mb.clamp(1, MAX_MB) << 20) / size_of::<Entry>();
        TranspositionTable { entries: vec![Entry::default(); len], age: 0 }
    }

    pub fn clear(&mut self) {
        self.entries.fill(Entry::default());
        self.age = 0;
    }

    /// Marks entries from earlier searches as replaceable.
    pub fn new_search(&mut self) {
        self.age = (self.age + 1) & 63;
    }

    #[inline(always)]
    fn index(&self, key: u64) -> usize {
        // Fixed-point multiply maps the key uniformly onto 0..len without a modulo.
        ((u128::from(key) * self.entries.len() as u128) >> 64) as usize
    }

    #[inline]
    pub fn probe(&self, key: u64) -> Option<Hit> {
        let e = self.entries[self.index(key)];
        let bound = match e.meta & 3 {
            1 => Bound::Upper,
            2 => Bound::Lower,
            3 => Bound::Exact,
            _ => return None,
        };
        (e.key == key).then(|| Hit {
            mv: Move::from_raw(e.mv),
            score: e.score.into(),
            depth: e.depth.into(),
            bound,
        })
    }

    /// `score` must already be converted to ply-independent form. A null `mv`
    /// keeps the move previously stored for the same position.
    #[inline]
    pub fn store(&mut self, key: u64, mv: Move, score: i32, depth: i32, bound: Bound) {
        let age = self.age;
        let index = self.index(key);
        let e = &mut self.entries[index];
        let replace = e.meta & 3 == 0
            || e.meta >> 2 != age
            || bound == Bound::Exact
            || depth + 3 >= i32::from(e.depth);
        if !replace {
            return;
        }
        let mv = if mv.is_null() && e.key == key { e.mv } else { mv.raw() };
        *e = Entry {
            key,
            mv,
            score: score as i16,
            depth: depth.clamp(0, u8::MAX.into()) as u8,
            meta: age << 2 | bound as u8,
        };
    }

    /// Permille of sampled slots filled by the current search (UCI `hashfull`).
    pub fn hashfull(&self) -> usize {
        let sample = &self.entries[..self.entries.len().min(1000)];
        let used = sample.iter().filter(|e| e.meta & 3 != 0 && e.meta >> 2 == self.age).count();
        used * 1000 / sample.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shallow_entry_from_same_search_does_not_evict_deep_one() {
        let mut tt = TranspositionTable::new(1);
        let deep = Move::from_raw(0x0123);
        tt.store(42, deep, 100, 12, Bound::Lower);
        tt.store(42, Move::from_raw(0x0456), -5, 2, Bound::Upper);
        let hit = tt.probe(42).unwrap();
        assert_eq!((hit.mv, hit.depth, hit.bound), (deep, 12, Bound::Lower));

        // After a new search the old entry is fair game.
        tt.new_search();
        tt.store(42, Move::NULL, -5, 2, Bound::Upper);
        let hit = tt.probe(42).unwrap();
        assert_eq!((hit.mv, hit.depth, hit.bound), (deep, 2, Bound::Upper), "null move keeps the stored one");
        assert!(tt.probe(43).is_none());
    }
}
