//! Legal move generation with check and pin masks: no make/test/unmake.

use crate::{
    attacks::{between, bishop_attacks, king_attacks, knight_attacks, line, pawn_attacks, rook_attacks},
    bitboard::Bitboard,
    board::{Board, CastleRights},
    moves::{Move, MoveList},
    types::{Color, PieceType, Square},
};

impl Board {
    /// Appends every legal move.
    #[inline]
    pub fn generate_moves(&self, list: &mut MoveList) {
        self.generate::<false>(list);
    }

    /// Appends legal captures (including en passant) and all promotions.
    #[inline]
    pub fn generate_noisy(&self, list: &mut MoveList) {
        self.generate::<true>(list);
    }

    /// Finds the legal move written in UCI notation (`e2e4`, `e7e8q`, `e1g1`).
    pub fn find_uci_move(&self, text: &str) -> Option<Move> {
        let from = Square::parse(text.get(0..2)?)?;
        let to = Square::parse(text.get(2..4)?)?;
        let promotion = match text.get(4..)? {
            "" => None,
            "n" => Some(PieceType::Knight),
            "b" => Some(PieceType::Bishop),
            "r" => Some(PieceType::Rook),
            "q" => Some(PieceType::Queen),
            _ => return None,
        };
        let mut list = MoveList::new();
        self.generate_moves(&mut list);
        list.iter().copied().find(|m| m.from() == from && m.to() == to && m.promotion() == promotion)
    }

    fn generate<const NOISY: bool>(&self, list: &mut MoveList) {
        let us = self.side;
        let them = !us;
        let occ = self.occupied();
        let ours = self.colors[us.index()];
        let theirs = self.colors[them.index()];
        let ksq = self.king_square(us);

        // King: target squares are checked with the king lifted, so it cannot
        // step backwards along a checking slider's ray.
        let king_targets = king_attacks(ksq) & !ours & if NOISY { theirs } else { Bitboard::FULL };
        let occ_without_king = occ ^ ksq.bb();
        for to in king_targets {
            if !self.is_attacked_by(to, them, occ_without_king) {
                let flag = if theirs.contains(to) { Move::CAPTURE } else { Move::QUIET };
                list.push(Move::new(ksq, to, flag));
            }
        }
        if self.checkers.more_than_one() {
            return;
        }

        // Non-king moves must capture the checker or block its ray.
        let check_mask = if self.checkers.any() {
            between(ksq, self.checkers.lsb()) | self.checkers
        } else {
            Bitboard::FULL
        };
        let targets = !ours & check_mask & if NOISY { theirs } else { Bitboard::FULL };

        if !NOISY && self.checkers.is_empty() {
            self.generate_castling(list, occ);
        }

        let pinned = self.pinned;
        for from in self.colored(us, PieceType::Knight) & !pinned {
            push_moves(list, from, knight_attacks(from) & targets, theirs);
        }
        let queens = self.pieces[PieceType::Queen.index()];
        for from in (self.pieces[PieceType::Bishop.index()] | queens) & ours {
            let mut attacks = bishop_attacks(from, occ) & targets;
            if pinned.contains(from) {
                attacks &= line(ksq, from);
            }
            push_moves(list, from, attacks, theirs);
        }
        for from in (self.pieces[PieceType::Rook.index()] | queens) & ours {
            let mut attacks = rook_attacks(from, occ) & targets;
            if pinned.contains(from) {
                attacks &= line(ksq, from);
            }
            push_moves(list, from, attacks, theirs);
        }

        self.generate_pawns::<NOISY>(list, ksq, occ, theirs, check_mask);
    }

    fn generate_castling(&self, list: &mut MoveList, occ: Bitboard) {
        let us = self.side;
        let them = !us;
        let base = if us == Color::White { 0 } else { 56 };
        let sq = |file: u8| Square::new(base + file);
        let king = sq(4);

        // Rights imply king and rook on their home squares (FEN sanitizing + CASTLE_KEEP).
        if self.castling.contains(CastleRights::kingside(us))
            && (occ & (sq(5).bb() | sq(6).bb())).is_empty()
            && !self.is_attacked_by(sq(5), them, occ)
            && !self.is_attacked_by(sq(6), them, occ)
        {
            list.push(Move::new(king, sq(6), Move::KING_CASTLE));
        }
        if self.castling.contains(CastleRights::queenside(us))
            && (occ & (sq(1).bb() | sq(2).bb() | sq(3).bb())).is_empty()
            && !self.is_attacked_by(sq(3), them, occ)
            && !self.is_attacked_by(sq(2), them, occ)
        {
            list.push(Move::new(king, sq(2), Move::QUEEN_CASTLE));
        }
    }

    fn generate_pawns<const NOISY: bool>(
        &self,
        list: &mut MoveList,
        ksq: Square,
        occ: Bitboard,
        theirs: Bitboard,
        check_mask: Bitboard,
    ) {
        let us = self.side;
        let white = us == Color::White;
        let pawns = self.colored(us, PieceType::Pawn);
        let (promo_rank, third_rank) =
            if white { (Bitboard::rank(7), Bitboard::rank(2)) } else { (Bitboard::rank(0), Bitboard::rank(5)) };
        // Square deltas: one step forward, capture towards file a, capture towards file h.
        let (up, west, east): (i8, i8, i8) = if white { (8, 7, 9) } else { (-8, -9, -7) };
        let forward = |bb: Bitboard| if white { bb << 8 } else { bb >> 8 };

        // Unpinned pawns, set-wise.
        let free = pawns & !self.pinned;
        let empty = !occ;
        let single = forward(free) & empty;
        let double = forward(single & third_rank) & empty & check_mask;
        let single = single & check_mask;
        let not_a = free & !Bitboard::file(0);
        let not_h = free & !Bitboard::file(7);
        let (west_caps, east_caps) = if white { (not_a << 7, not_h << 9) } else { (not_a >> 9, not_h >> 7) };
        let west_caps = west_caps & theirs & check_mask;
        let east_caps = east_caps & theirs & check_mask;

        for to in single & promo_rank {
            push_promotions(list, to.offset(-up), to, false);
        }
        for to in west_caps & promo_rank {
            push_promotions(list, to.offset(-west), to, true);
        }
        for to in east_caps & promo_rank {
            push_promotions(list, to.offset(-east), to, true);
        }
        for to in west_caps & !promo_rank {
            list.push(Move::new(to.offset(-west), to, Move::CAPTURE));
        }
        for to in east_caps & !promo_rank {
            list.push(Move::new(to.offset(-east), to, Move::CAPTURE));
        }
        if !NOISY {
            for to in single & !promo_rank {
                list.push(Move::new(to.offset(-up), to, Move::QUIET));
            }
            for to in double {
                list.push(Move::new(to.offset(-2 * up), to, Move::DOUBLE_PUSH));
            }
        }

        // Pinned pawns may only move along the pin line.
        for from in pawns & self.pinned {
            let allowed = line(ksq, from) & check_mask;
            for to in pawn_attacks(us, from) & theirs & allowed {
                if promo_rank.contains(to) {
                    push_promotions(list, from, to, true);
                } else {
                    list.push(Move::new(from, to, Move::CAPTURE));
                }
            }
            let one = from.offset(up);
            if occ.contains(one) || !allowed.contains(one) {
                continue;
            }
            if promo_rank.contains(one) {
                push_promotions(list, from, one, false);
            } else if !NOISY {
                list.push(Move::new(from, one, Move::QUIET));
                let two = one.offset(up);
                if third_rank.contains(one) && !occ.contains(two) && allowed.contains(two) {
                    list.push(Move::new(from, two, Move::DOUBLE_PUSH));
                }
            }
        }

        // En passant: rare, so verify legality by simulating the capture. This
        // covers the horizontal discovered check through both pawns.
        if let Some(ep) = self.ep {
            let captured = Square::new(ep.0 ^ 8);
            let [enemy_pawns, knights, bishops, rooks, queens, _] = self.pieces;
            let theirs_after = theirs ^ captured.bb();
            for from in pawn_attacks(!us, ep) & pawns {
                let occ_after = (occ ^ from.bb() ^ captured.bb()) | ep.bb();
                let attacked = (rook_attacks(ksq, occ_after) & (rooks | queens) & theirs).any()
                    || (bishop_attacks(ksq, occ_after) & (bishops | queens) & theirs).any()
                    || (knight_attacks(ksq) & knights & theirs).any()
                    || (pawn_attacks(us, ksq) & enemy_pawns & theirs_after).any();
                if !attacked {
                    list.push(Move::new(from, ep, Move::EN_PASSANT));
                }
            }
        }
    }
}

#[inline(always)]
fn push_moves(list: &mut MoveList, from: Square, targets: Bitboard, theirs: Bitboard) {
    for to in targets & theirs {
        list.push(Move::new(from, to, Move::CAPTURE));
    }
    for to in targets & !theirs {
        list.push(Move::new(from, to, Move::QUIET));
    }
}

/// Queen first: it is almost always the best promotion.
#[inline(always)]
fn push_promotions(list: &mut MoveList, from: Square, to: Square, capture: bool) {
    let base = if capture { Move::PROMOTION_CAPTURE } else { Move::PROMOTION };
    for piece in [3, 0, 2, 1] {
        list.push(Move::new(from, to, base | piece));
    }
}
