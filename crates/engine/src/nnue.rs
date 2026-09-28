//! NNUE evaluation: `(768 -> HIDDEN)x2 -> 1` with SCReLU, as trained by
//! `trainer/` (bullet). The network is embedded from `nets/current.bin`.
//!
//! Two accumulators per position, one per perspective. Feature of a piece of
//! color `C`, type `K` on square `s` seen from perspective `P`:
//! `(C != P) * 384 + K * 64 + (P == black ? s ^ 56 : s)`.

use std::sync::LazyLock;

use movegen::{Board, Color, Move, Piece, PieceType, Square};

/// Must match `trainer`'s `HIDDEN`, `SCALE`, `QA`, `QB`.
pub const HIDDEN: usize = 256;
const SCALE: i32 = 400;
const QA: i32 = 255;
const QB: i32 = 64;

/// bullet's quantised output, byte-for-byte (little-endian i16, padded to 64 bytes).
#[repr(C)]
struct Network {
    feature_weights: [Accumulator; 768],
    feature_bias: Accumulator,
    /// Side to move's half first.
    output_weights: [i16; 2 * HIDDEN],
    output_bias: i16,
}

static NET_BYTES: &[u8] = include_bytes!("../../../nets/current.bin");

static NET: LazyLock<Box<Network>> = LazyLock::new(|| {
    assert_eq!(NET_BYTES.len(), size_of::<Network>(), "nets/current.bin does not match HIDDEN = {HIDDEN}");
    let mut net = Box::<Network>::new_uninit();
    // SAFETY: `Network` is plain `i16` data where every bit pattern is valid, the
    // lengths are equal, and the target is a fresh allocation.
    let net = unsafe {
        std::ptr::copy_nonoverlapping(NET_BYTES.as_ptr(), net.as_mut_ptr().cast::<u8>(), NET_BYTES.len());
        net.assume_init()
    };
    // The AVX2 output layer squares the clipped activation in 16-bit lanes, so
    // `weight * QA` must not overflow.
    #[cfg(target_feature = "avx2")]
    assert!(
        net.output_weights.iter().all(|w| i32::from(*w).abs() * QA <= i32::from(i16::MAX)),
        "l1w values are too large for the AVX2 evaluation path"
    );
    net
});

/// Hidden-layer pre-activations of one perspective.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C, align(64))]
pub struct Accumulator {
    vals: [i16; HIDDEN],
}

/// Both perspectives, indexed by `Color`.
pub type Accumulators = [Accumulator; 2];

#[inline(always)]
fn feature(perspective: Color, piece: Piece, sq: Square) -> usize {
    let flip = if perspective == Color::Black { 56 } else { 0 };
    usize::from(piece.color() != perspective) * 384 + piece.kind().index() * 64 + (sq.index() ^ flip)
}

/// Accumulators computed from scratch.
pub fn refresh(board: &Board) -> Accumulators {
    let net = &*NET;
    let mut accs = [net.feature_bias; 2];
    for (perspective, acc) in [Color::White, Color::Black].into_iter().zip(&mut accs) {
        for sq in board.occupied() {
            let piece = board.piece_at(sq).expect("occupied square has a piece");
            let column = &net.feature_weights[feature(perspective, piece, sq)];
            for (v, w) in acc.vals.iter_mut().zip(&column.vals) {
                *v += w;
            }
        }
    }
    accs
}

/// Accumulators after `mv`, derived from `parent` (the accumulators of `board`).
/// `board` is the position before the move.
pub fn update(parent: &Accumulators, board: &Board, mv: Move) -> Accumulators {
    let (from, to) = (mv.from(), mv.to());
    let piece = board.piece_at(from).expect("move starts on a piece");
    let us = piece.color();
    let placed = mv.promotion().map_or(piece, |kind| Piece::new(us, kind));

    // At most two features appear and two disappear.
    let mut adds = [(placed, to), (placed, to)];
    let mut subs = [(piece, from), (piece, from)];
    let (mut n_add, mut n_sub) = (1, 1);
    if mv.is_en_passant() {
        subs[1] = (Piece::new(!us, PieceType::Pawn), Square::new((to.index() ^ 8) as u8));
        n_sub = 2;
    } else if let Some(captured) = board.piece_at(to) {
        subs[1] = (captured, to);
        n_sub = 2;
    } else if mv.is_castle() {
        let rook = Piece::new(us, PieceType::Rook);
        let base = from.index() as u8 & 56;
        let (rook_from, rook_to) = if to.file() == 6 { (base + 7, base + 5) } else { (base, base + 3) };
        adds[1] = (rook, Square::new(rook_to));
        subs[1] = (rook, Square::new(rook_from));
        (n_add, n_sub) = (2, 2);
    }

    let net = &*NET;
    let mut out = *parent;
    for (perspective, acc) in [Color::White, Color::Black].into_iter().zip(&mut out) {
        let column = |p: (Piece, Square)| &net.feature_weights[feature(perspective, p.0, p.1)];
        let (a0, s0) = (column(adds[0]), column(subs[0]));
        match (n_add, n_sub) {
            (1, 1) => add_sub(acc, a0, s0),
            (1, 2) => add_sub2(acc, a0, s0, column(subs[1])),
            _ => add_sub4(acc, a0, column(adds[1]), s0, column(subs[1])),
        }
    }
    out
}

/// `acc += add - sub`.
#[inline(always)]
fn add_sub(acc: &mut Accumulator, add: &Accumulator, sub: &Accumulator) {
    #[cfg(target_feature = "avx2")]
    // SAFETY: AVX2 is enabled for the whole build (cfg above).
    unsafe {
        avx2::add_sub(acc, add, sub);
    }
    #[cfg(not(target_feature = "avx2"))]
    for i in 0..HIDDEN {
        acc.vals[i] += add.vals[i] - sub.vals[i];
    }
}

/// `acc += add - sub0 - sub1` (en passant and ordinary captures).
#[inline(always)]
fn add_sub2(acc: &mut Accumulator, add: &Accumulator, sub0: &Accumulator, sub1: &Accumulator) {
    #[cfg(target_feature = "avx2")]
    // SAFETY: AVX2 is enabled for the whole build (cfg above).
    unsafe {
        avx2::add_sub2(acc, add, sub0, sub1);
    }
    #[cfg(not(target_feature = "avx2"))]
    for i in 0..HIDDEN {
        acc.vals[i] += add.vals[i] - sub0.vals[i] - sub1.vals[i];
    }
}

/// `acc += add0 + add1 - sub0 - sub1` (castling).
#[inline(always)]
fn add_sub4(acc: &mut Accumulator, add0: &Accumulator, add1: &Accumulator, sub0: &Accumulator, sub1: &Accumulator) {
    #[cfg(target_feature = "avx2")]
    // SAFETY: AVX2 is enabled for the whole build (cfg above).
    unsafe {
        avx2::add_sub4(acc, add0, add1, sub0, sub1);
    }
    #[cfg(not(target_feature = "avx2"))]
    for i in 0..HIDDEN {
        acc.vals[i] += add0.vals[i] + add1.vals[i] - sub0.vals[i] - sub1.vals[i];
    }
}

/// Score in centipawns for the side to move.
pub fn evaluate(accs: &Accumulators, side_to_move: Color) -> i32 {
    let net = &*NET;
    let us = &accs[side_to_move.index()];
    let them = &accs[(!side_to_move).index()];
    let (w_us, w_them) = net.output_weights.split_at(HIDDEN);
    #[cfg(target_feature = "avx2")]
    // SAFETY: AVX2 is enabled for the whole build (cfg above).
    let output = unsafe { avx2::dot(us, them, w_us.try_into().unwrap(), w_them.try_into().unwrap()) };
    #[cfg(not(target_feature = "avx2"))]
    let output = dot(us, them, w_us, w_them);
    (output / QA + i32::from(net.output_bias)) * SCALE / (QA * QB)
}

/// `Σ screlu(us) * w_us + Σ screlu(them) * w_them`, on the `QA² * QB` scale.
/// Scalar reference: used when the build has no AVX2, and by the equivalence test.
#[cfg_attr(target_feature = "avx2", allow(dead_code, reason = "scalar reference path"))]
fn dot(us: &Accumulator, them: &Accumulator, w_us: &[i16], w_them: &[i16]) -> i32 {
    let mut output = 0i32;
    for i in 0..HIDDEN {
        output += screlu(us.vals[i]) * i32::from(w_us[i]) + screlu(them.vals[i]) * i32::from(w_them[i]);
    }
    output
}

/// Squared clipped ReLU on the quantised scale: `clamp(x, 0, QA)^2`.
#[cfg_attr(target_feature = "avx2", allow(dead_code, reason = "scalar reference path"))]
#[inline(always)]
fn screlu(x: i16) -> i32 {
    let y = i32::from(x).clamp(0, QA);
    y * y
}

/// AVX2 kernels. `Accumulator` is 64-byte aligned, so the loads stay aligned.
#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::*;

    use super::{Accumulator, HIDDEN, QA};

    /// `i16` lanes per `__m256i`.
    const LANES: usize = 16;
    const VECTORS: usize = HIDDEN / LANES;

    #[target_feature(enable = "avx2")]
    pub unsafe fn add_sub(acc: &mut Accumulator, add: &Accumulator, sub: &Accumulator) {
        let (acc, add, sub) = (vec_ptr(acc), vec_ptr_const(add), vec_ptr_const(sub));
        for i in 0..VECTORS {
            // SAFETY: all three point to `HIDDEN` lanes; every pointer is 32-byte aligned.
            unsafe {
                let v = _mm256_sub_epi16(_mm256_load_si256(acc.add(i)), _mm256_load_si256(sub.add(i)));
                _mm256_store_si256(acc.add(i), _mm256_add_epi16(v, _mm256_load_si256(add.add(i))));
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn add_sub2(acc: &mut Accumulator, add: &Accumulator, sub0: &Accumulator, sub1: &Accumulator) {
        let (acc, add, sub0, sub1) = (vec_ptr(acc), vec_ptr_const(add), vec_ptr_const(sub0), vec_ptr_const(sub1));
        for i in 0..VECTORS {
            // SAFETY: as in `add_sub`.
            unsafe {
                let v = _mm256_sub_epi16(_mm256_load_si256(acc.add(i)), _mm256_load_si256(sub0.add(i)));
                let v = _mm256_sub_epi16(v, _mm256_load_si256(sub1.add(i)));
                _mm256_store_si256(acc.add(i), _mm256_add_epi16(v, _mm256_load_si256(add.add(i))));
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn add_sub4(
        acc: &mut Accumulator,
        add0: &Accumulator,
        add1: &Accumulator,
        sub0: &Accumulator,
        sub1: &Accumulator,
    ) {
        let (acc, add0, add1) = (vec_ptr(acc), vec_ptr_const(add0), vec_ptr_const(add1));
        let (sub0, sub1) = (vec_ptr_const(sub0), vec_ptr_const(sub1));
        for i in 0..VECTORS {
            // SAFETY: as in `add_sub`.
            unsafe {
                let v = _mm256_sub_epi16(_mm256_load_si256(acc.add(i)), _mm256_load_si256(sub0.add(i)));
                let v = _mm256_sub_epi16(v, _mm256_load_si256(sub1.add(i)));
                let v = _mm256_add_epi16(v, _mm256_load_si256(add0.add(i)));
                _mm256_store_si256(acc.add(i), _mm256_add_epi16(v, _mm256_load_si256(add1.add(i))));
            }
        }
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn dot(us: &Accumulator, them: &Accumulator, w_us: &[i16; HIDDEN], w_them: &[i16; HIDDEN]) -> i32 {
        let zero = _mm256_setzero_si256();
        let qa = _mm256_set1_epi16(QA as i16);
        let mut sum = _mm256_setzero_si256();
        for i in 0..VECTORS {
            // SAFETY: accumulators hold `HIDDEN` lanes and are 32-byte aligned;
            // the weight arrays are `HIDDEN` long and read unaligned.
            unsafe {
                let a = _mm256_load_si256(vec_ptr_const(us).add(i));
                let wa = _mm256_loadu_si256(w_us.as_ptr().add(i * LANES).cast::<__m256i>());
                let clamped = _mm256_min_epi16(_mm256_max_epi16(a, zero), qa);
                sum = _mm256_add_epi32(sum, _mm256_madd_epi16(clamped, _mm256_mullo_epi16(clamped, wa)));

                let b = _mm256_load_si256(vec_ptr_const(them).add(i));
                let wb = _mm256_loadu_si256(w_them.as_ptr().add(i * LANES).cast::<__m256i>());
                let clamped = _mm256_min_epi16(_mm256_max_epi16(b, zero), qa);
                sum = _mm256_add_epi32(sum, _mm256_madd_epi16(clamped, _mm256_mullo_epi16(clamped, wb)));
            }
        }
        let mut lanes = [0i32; 8];
        // SAFETY: exactly 8 `i32` lanes fit the 256-bit register.
        unsafe { _mm256_storeu_si256(lanes.as_mut_ptr().cast::<__m256i>(), sum) };
        lanes.iter().sum()
    }

    /// Pointer to the accumulator's lanes as AVX2 vectors.
    #[inline(always)]
    fn vec_ptr(acc: &mut Accumulator) -> *mut __m256i {
        acc.vals.as_mut_ptr().cast::<__m256i>()
    }

    /// Read-only variant of `vec_ptr`.
    #[inline(always)]
    fn vec_ptr_const(acc: &Accumulator) -> *const __m256i {
        acc.vals.as_ptr().cast::<__m256i>()
    }
}

#[cfg(test)]
mod tests {
    use movegen::MoveList;

    use super::*;

    /// Incremental updates must equal a full refresh for every move type
    /// (captures, en passant, castling, promotions) along random games.
    #[test]
    fn incremental_update_matches_refresh() {
        let mut seed = 0x1234_5678_9ABC_DEF1u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let fens = [
            movegen::START_FEN,
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
        ];
        let mut checked = 0;
        for fen in fens {
            for _ in 0..30 {
                let mut board = Board::from_fen(fen).unwrap();
                let mut accs = refresh(&board);
                for _ in 0..80 {
                    let mut moves = MoveList::new();
                    board.generate_moves(&mut moves);
                    if moves.is_empty() {
                        break;
                    }
                    let mv = moves[(next() % moves.len() as u64) as usize];
                    accs = update(&accs, &board, mv);
                    board.make_move(mv);
                    assert_eq!(accs, refresh(&board), "after {mv}: {}", board.to_fen());
                    checked += 1;
                }
            }
        }
        assert!(checked > 5_000);
    }

    /// The AVX2 output layer must agree with the scalar reference exactly.
    #[cfg(target_feature = "avx2")]
    #[test]
    fn avx2_output_layer_matches_scalar() {
        let net = &*NET;
        let (w_us, w_them) = net.output_weights.split_at(HIDDEN);
        let (w_us, w_them): (&[i16; HIDDEN], &[i16; HIDDEN]) =
            (w_us.try_into().unwrap(), w_them.try_into().unwrap());
        let mut seed = 0xDEAD_BEEF_1234_5678u64;
        let mut board = Board::startpos();
        let mut checked = 0;
        for _ in 0..2_000 {
            let mut moves = MoveList::new();
            board.generate_moves(&mut moves);
            if moves.is_empty() {
                board = Board::startpos();
                continue;
            }
            let accs = refresh(&board);
            for side in [Color::White, Color::Black] {
                let (us, them) = (&accs[side.index()], &accs[(!side).index()]);
                let scalar = dot(us, them, w_us, w_them);
                // SAFETY: AVX2 is enabled for this build (cfg above).
                let simd = unsafe { avx2::dot(us, them, w_us, w_them) };
                assert_eq!(scalar, simd, "{}", board.to_fen());
                checked += 1;
            }
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mv = moves[(seed % moves.len() as u64) as usize];
            if board.piece_at(mv.to()).is_some() || mv.is_promotion() || mv.is_en_passant() {
                board = Board::startpos();
            } else {
                board.make_move(mv);
            }
        }
        assert!(checked > 1_000);
    }

    /// Mirroring the board and swapping colors must not change the evaluation.
    #[test]
    fn evaluation_is_color_symmetric() {
        let white = Board::from_fen("r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4").unwrap();
        let black = Board::from_fen("rnbqk2r/pppp1ppp/5n2/2b1p3/4P3/2N2N2/PPPP1PPP/R1BQKB1R b KQkq - 4 4").unwrap();
        assert_eq!(evaluate(&refresh(&white), Color::White), evaluate(&refresh(&black), Color::Black));
    }
}
