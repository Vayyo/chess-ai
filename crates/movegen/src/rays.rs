//! Slider ray walking shared by `build.rs` (attack-table generation) and the
//! crate's const tables. Dependency-free so the build script can include it
//! via `#[path]`.

/// (file delta, rank delta) steps of orthogonal sliders.
pub const ROOK_DIRS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
/// (file delta, rank delta) steps of diagonal sliders.
pub const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

const fn on_board(file: i8, rank: i8) -> bool {
    file >= 0 && file < 8 && rank >= 0 && rank < 8
}

/// Squares attacked from `sq` along `dirs`; each ray stops at (and includes)
/// the first occupied square of `occ`.
pub const fn ray_attacks(sq: u8, occ: u64, dirs: &[(i8, i8); 4]) -> u64 {
    let mut attacks = 0u64;
    let mut d = 0;
    while d < dirs.len() {
        let (df, dr) = dirs[d];
        let mut file = (sq % 8) as i8 + df;
        let mut rank = (sq / 8) as i8 + dr;
        while on_board(file, rank) {
            let bit = 1u64 << (rank * 8 + file);
            attacks |= bit;
            if occ & bit != 0 {
                break;
            }
            file += df;
            rank += dr;
        }
        d += 1;
    }
    attacks
}

/// Squares whose occupancy can change `ray_attacks(sq, _, dirs)`: every ray
/// square except the last one before the board edge.
#[allow(dead_code, reason = "used by build.rs only")]
pub const fn relevant_mask(sq: u8, dirs: &[(i8, i8); 4]) -> u64 {
    let mut mask = 0u64;
    let mut d = 0;
    while d < dirs.len() {
        let (df, dr) = dirs[d];
        let mut file = (sq % 8) as i8 + df;
        let mut rank = (sq / 8) as i8 + dr;
        while on_board(file + df, rank + dr) {
            mask |= 1u64 << (rank * 8 + file);
            file += df;
            rank += dr;
        }
        d += 1;
    }
    mask
}
