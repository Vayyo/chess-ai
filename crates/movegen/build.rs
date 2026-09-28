//! Generates the slider attack tables into `OUT_DIR`.
//!
//! x86_64 with BMI2 enabled: blocks are indexed by `pext(occupancy, mask)`.
//! Otherwise: fancy magic bitboards, magics searched here with a fixed seed.
//! Output: `slider_attacks.bin` (target-endian u64 attack sets) and
//! `slider_tables.rs` (per-square `Magic` entries + table length).

#[path = "src/rays.rs"]
mod rays;

use std::{env, fmt::Write as _, fs, path::PathBuf};

struct Entry {
    mask: u64,
    magic: u64,
    shift: u32,
    offset: usize,
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src/rays.rs");
    println!("cargo::rerun-if-env-changed=MOVEGEN_NO_PEXT");
    println!("cargo::rustc-check-cfg=cfg(movegen_pext)");

    // Single source of truth for the table layout: the crate indexes with PEXT
    // exactly when `movegen_pext` is set. MOVEGEN_NO_PEXT forces magics on CPUs
    // with microcoded PEXT (AMD Zen 1/2).
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let features = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
    let use_pext = arch == "x86_64"
        && features.split(',').any(|f| f == "bmi2")
        && env::var_os("MOVEGEN_NO_PEXT").is_none();
    if use_pext {
        println!("cargo::rustc-cfg=movegen_pext");
    }
    let big_endian = env::var("CARGO_CFG_TARGET_ENDIAN").as_deref() == Ok("big");

    let mut table = Vec::new();
    let mut rng = XorShift64(0x2545_F491_4F6C_DD1D);
    let rook = build(&rays::ROOK_DIRS, use_pext, &mut table, &mut rng);
    let bishop = build(&rays::BISHOP_DIRS, use_pext, &mut table, &mut rng);

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let bytes: Vec<u8> = table
        .iter()
        .flat_map(|v| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() })
        .collect();
    fs::write(out.join("slider_attacks.bin"), bytes).expect("write slider_attacks.bin");

    let mut src = String::new();
    writeln!(src, "pub(crate) const SLIDER_TABLE_LEN: usize = {};", table.len()).unwrap();
    write_entries(&mut src, "ROOK_MAGICS", &rook);
    write_entries(&mut src, "BISHOP_MAGICS", &bishop);
    fs::write(out.join("slider_tables.rs"), src).expect("write slider_tables.rs");
}

fn write_entries(src: &mut String, name: &str, entries: &[Entry]) {
    writeln!(src, "pub(crate) static {name}: [Magic; 64] = [").unwrap();
    for e in entries {
        writeln!(
            src,
            "    Magic {{ mask: {:#018x}, magic: {:#018x}, shift: {}, offset: {} }},",
            e.mask, e.magic, e.shift, e.offset
        )
        .unwrap();
    }
    writeln!(src, "];").unwrap();
}

fn build(dirs: &[(i8, i8); 4], use_pext: bool, table: &mut Vec<u64>, rng: &mut XorShift64) -> Vec<Entry> {
    (0..64u8)
        .map(|sq| {
            let mask = rays::relevant_mask(sq, dirs);
            let bits = mask.count_ones();
            let offset = table.len();
            let (occupancies, attacks): (Vec<u64>, Vec<u64>) =
                subsets(mask).map(|occ| (occ, rays::ray_attacks(sq, occ, dirs))).unzip();
            table.resize(offset + (1 << bits), 0);
            let block = &mut table[offset..];
            if use_pext {
                for (&occ, &att) in occupancies.iter().zip(&attacks) {
                    block[pext(occ, mask) as usize] = att;
                }
                Entry { mask, magic: 0, shift: 0, offset }
            } else {
                let shift = 64 - bits;
                let magic = find_magic(mask, shift, &occupancies, &attacks, rng);
                for (&occ, &att) in occupancies.iter().zip(&attacks) {
                    block[(occ.wrapping_mul(magic) >> shift) as usize] = att;
                }
                Entry { mask, magic, shift, offset }
            }
        })
        .collect()
}

/// Every subset of `mask` (carry-rippler), starting with the empty set.
fn subsets(mask: u64) -> impl Iterator<Item = u64> {
    let mut next = Some(0u64);
    std::iter::from_fn(move || {
        let current = next?;
        let n = current.wrapping_sub(mask) & mask;
        next = (n != 0).then_some(n);
        Some(current)
    })
}

/// Software parallel bit extract; must match `_pext_u64` bit order.
fn pext(src: u64, mut mask: u64) -> u64 {
    let mut result = 0;
    let mut bit = 1;
    while mask != 0 {
        let lowest = mask.isolate_lowest_one();
        if src & lowest != 0 {
            result |= bit;
        }
        mask ^= lowest;
        bit <<= 1;
    }
    result
}

/// Magic with no destructive collisions: colliding occupancies must share the attack set.
fn find_magic(mask: u64, shift: u32, occupancies: &[u64], attacks: &[u64], rng: &mut XorShift64) -> u64 {
    let size = occupancies.len();
    let mut used = vec![0u64; size];
    let mut stamp = vec![0u32; size];
    let mut attempt = 0u32;
    loop {
        let magic = rng.sparse();
        if (mask.wrapping_mul(magic) >> 56).count_ones() < 6 {
            continue;
        }
        attempt += 1;
        let ok = occupancies.iter().zip(attacks).all(|(&occ, &att)| {
            let i = (occ.wrapping_mul(magic) >> shift) as usize;
            if stamp[i] != attempt {
                stamp[i] = attempt;
                used[i] = att;
                true
            } else {
                used[i] == att
            }
        });
        if ok {
            return magic;
        }
    }
}

struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn sparse(&mut self) -> u64 {
        self.next() & self.next() & self.next()
    }
}
