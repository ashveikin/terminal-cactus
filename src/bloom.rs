//! The bloom finale: after the last column caps, every tracked tip may
//! sprout a flower, one after another, like a real cactus putting out its
//! crown at the end of the season.
//!
//! All flower decisions are rolled from the single RNG stream in a fixed
//! order before anything is drawn, so live, print and saved replays bloom
//! identically; live mode merely inserts a bud frame and pacing between the
//! two frames per tip.

use crate::ctx::Ctx;
use crate::output::{
    PAIR_FLOWER_PINK, PAIR_FLOWER_RED, PAIR_FLOWER_WHITE, PAIR_FLOWER_YELLOW,
    PAIR_LIGHT_SPINE,
};
use crate::rand::GlibcRand;

/// A column tip that may bloom, recorded by the growth engine.
pub struct Tip {
    pub y: i32,
    pub x: i32,
    /// Trunk crown: eligible for the big multi-bloom shapes.
    pub crown: bool,
}

struct Flower {
    /// (dx, codepoint, pair, bold) relative to the tip.
    cells: Vec<(i32, u32, u8, bool)>,
}

pub struct BloomState {
    tips: Vec<Tip>,
    flowers: Vec<Option<Flower>>,
    decided: bool,
    idx: usize,
    bud_phase: bool,
}

pub enum BloomStep {
    Drew,
    Done,
}

impl BloomState {
    pub fn new(tips: Vec<Tip>) -> BloomState {
        BloomState { tips, flowers: Vec::new(), decided: false, idx: 0, bud_phase: false }
    }
}

fn petal_pair(rng: &mut GlibcRand) -> u8 {
    const PALETTE: [u8; 4] =
        [PAIR_FLOWER_PINK, PAIR_FLOWER_RED, PAIR_FLOWER_YELLOW, PAIR_FLOWER_WHITE];
    PALETTE[(rng.rand() % 4) as usize]
}

fn make_flower(rng: &mut GlibcRand, tip: &Tip) -> Flower {
    let petals = petal_pair(rng);
    let center_pair = if petals == PAIR_FLOWER_YELLOW { PAIR_FLOWER_WHITE } else { PAIR_FLOWER_YELLOW };
    let center = [b'@', b'o', b'*'][(rng.rand() % 3) as usize] as u32;

    // Trunk crowns sometimes carry a three-bloom band across the cap, the
    // `|o|o|` motif from the reference art; arms get single blossoms.
    if tip.crown && rng.rand() % 2 == 0 {
        let mut cells = Vec::new();
        for dx in [-2i32, 0, 2] {
            cells.push((dx, b'|' as u32, PAIR_LIGHT_SPINE, false));
        }
        for dx in [-1i32, 1] {
            cells.push((dx, center, petals, true));
        }
        return Flower { cells };
    }
    if !tip.crown && rng.rand() % 3 == 0 {
        // small single bloom
        return Flower { cells: vec![(0, center, petals, true)] };
    }
    let open = if rng.rand() % 2 == 0 { b'(' as u32 } else { b'{' as u32 };
    let close = if open == b'(' as u32 { b')' as u32 } else { b'}' as u32 };
    Flower {
        cells: vec![(-1, open, petals, true), (0, center, center_pair, true), (1, close, petals, true)],
    }
}

/// Decide every tip's fate up front (fixed RNG order, seed-reproducible).
fn decide(state: &mut BloomState, rng: &mut GlibcRand) {
    let mut any = false;
    let tips = std::mem::take(&mut state.tips);
    for tip in &tips {
        // crown tips almost always bloom; arm tips usually do
        let threshold = if tip.crown { 9 } else { 8 };
        let flower = if rng.rand() % 10 < threshold { Some(make_flower(rng, tip)) } else { None };
        any |= flower.is_some();
        state.flowers.push(flower);
    }
    state.tips = tips;
    if !any && !state.tips.is_empty() {
        // guarantee at least one bloom: the crown tip, else the last tip
        let pos = state
            .tips
            .iter()
            .rposition(|t| t.crown)
            .unwrap_or(state.tips.len() - 1);
        let flower = make_flower(rng, &state.tips[pos]);
        state.flowers[pos] = Some(flower);
    }
    state.decided = true;
}

/// One bloom animation frame. Each flowering tip contributes two frames
/// (bud, then full flower); tips without a flower are skipped instantly.
pub fn step(state: &mut BloomState, ctx: &mut Ctx) -> BloomStep {
    if !state.decided {
        decide(state, &mut ctx.rng);
        if state.tips.is_empty() {
            return BloomStep::Done;
        }
    }
    while state.idx < state.tips.len() {
        if state.flowers[state.idx].is_none() {
            state.idx += 1;
            continue;
        }
        let tip = &state.tips[state.idx];
        if !state.bud_phase {
            ctx.put_cell(tip.y, tip.x, b'o' as u32, true, PAIR_FLOWER_WHITE);
            state.bud_phase = true;
            return BloomStep::Drew;
        }
        let flower = state.flowers[state.idx].as_ref().unwrap();
        for &(dx, cp, pair, bold) in &flower.cells {
            ctx.put_cell(tip.y, tip.x + dx, cp, bold, pair);
        }
        state.idx += 1;
        state.bud_phase = false;
        return BloomStep::Drew;
    }
    BloomStep::Done
}
