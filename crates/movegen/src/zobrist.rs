//! Zobrist keys, generated at compile time from a fixed SplitMix64 seed so
//! hashes are stable across builds (required for persisted data).

pub(crate) struct Keys {
    pub pieces: [[u64; 64]; 12],
    /// Indexed by the 4-bit castling-rights set.
    pub castling: [u64; 16],
    pub ep_file: [u64; 8],
    /// XORed in when black is to move.
    pub side: u64,
}

pub(crate) static KEYS: Keys = {
    let mut state = 0x0C0F_FEE0_D15E_A5E5;
    let mut pieces = [[0; 64]; 12];
    let mut p = 0;
    while p < 12 {
        let mut sq = 0;
        while sq < 64 {
            pieces[p][sq] = splitmix64(&mut state);
            sq += 1;
        }
        p += 1;
    }
    let mut castling = [0; 16];
    let mut i = 0;
    while i < 16 {
        castling[i] = splitmix64(&mut state);
        i += 1;
    }
    let mut ep_file = [0; 8];
    let mut i = 0;
    while i < 8 {
        ep_file[i] = splitmix64(&mut state);
        i += 1;
    }
    Keys { pieces, castling, ep_file, side: splitmix64(&mut state) }
};

const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
