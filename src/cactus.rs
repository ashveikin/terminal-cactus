//! The cactus growth engine.
//!
//! A cactus is a set of columns (a trunk plus arms) growing in round-robin:
//! each animation step draws one horizontal slice of one column, so live
//! mode shows the trunk and its arms rising together. Arms sprout from the
//! trunk sides, lean outward diagonally, then straighten into their own
//! vertical columns — the classic saguaro silhouette of the reference art.
//! When a column tops out it is closed with a shoulder cap (`\d...b/`) and
//! its tip is recorded for the bloom finale.
//!
//! Every random decision — silhouette style, widths, sway, arm spawns,
//! texture cells, spines — comes from the single glibc-compatible stream in
//! a fixed order, which makes each seed's cactus reproducible across live,
//! print, save/load and resize replay.

use crate::bloom::Tip;
use crate::config::Config;
use crate::ctx::Ctx;
use crate::output::{PAIR_DARK_BODY, PAIR_DARK_SPINE, PAIR_LIGHT_BODY, PAIR_LIGHT_SPINE};
use crate::rand::GlibcRand;
use crate::screen::Window;
use crate::utf8::decode_one;

pub enum GrowthResult {
    Complete,
    Resize,
    /// checkKeyPress saw a quit key: finish (refresh frame, save) and exit.
    Quit,
}

pub enum StepOutcome {
    Drew,
    Done,
}

/// Body texture: ribs (`|`) dominate with organic noise, as in both
/// reference artworks.
const BODY_TEX: &[(u8, i32)] =
    &[(b'|', 55), (b';', 10), (b':', 9), (b'#', 9), (b'H', 8), (b'%', 5), (b'=', 4)];
/// Edge bias for the plain silhouette style: rim cells favor the spiky,
/// ribbed characters that outline the cactus in the reference art.
const EDGE_TEX: &[(u8, i32)] =
    &[(b'|', 28), (b'H', 15), (b'#', 15), (b'%', 12), (b'=', 12), (b'!', 9), (b':', 9)];

/// Lowest row a slice may occupy; the cap closes at `CEIL_Y - 1` and the
/// flower opens one row above that.
const CEIL_Y: i32 = 3;

fn tex_roll(rng: &mut GlibcRand, table: &[(u8, i32)]) -> u8 {
    let total: i32 = table.iter().map(|t| t.1).sum();
    let mut v = rng.rand() % total;
    for (ch, weight) in table {
        if v < *weight {
            return *ch;
        }
        v -= *weight;
    }
    table[table.len() - 1].0
}

fn spine_chars(conf: &Config) -> Vec<u32> {
    let mut out = Vec::new();
    for s in &conf.spines {
        if let Ok((cp, _)) = decode_one(s) {
            if cp != 0 {
                out.push(cp);
            }
        }
    }
    out
}

struct Column {
    trunk: bool,
    /// -1 = leans left, +1 = leans right (arms only).
    side: i32,
    /// Left edge of the current slice.
    l: i32,
    w: i32,
    /// Row the current slice is drawn on; climbing decreases it.
    y: i32,
    rows_left: i32,
    total_rows: i32,
    /// Remaining drift-out steps (arms).
    drift_rows: i32,
    /// Drift once per row (steep arm) or every other row (shallow ramp).
    drift_every: i32,
    drift_tick: i32,
    w_min: i32,
    w_max: i32,
    started: bool,
    prev_l: i32,
    prev_w: i32,
}

pub struct Engine {
    queue: Vec<Column>,
    /// Body-cell occupancy of the tree window (drives spine placement and
    /// merged-arm detection).
    occ: Vec<bool>,
    win_w: i32,
    win_h: i32,
    x_center0: i32,
    /// Pre-planned arm schedule: (sprout row, side) in bloom-to-climb order.
    /// Planning the arms up front (instead of dice per row) guarantees every
    /// seed a well-proportioned plant.
    arm_plan: Vec<(i32, i32)>,
    next_arm: usize,
    /// Framed slices (`._|||_.` on every wide row) vs plain texture edges.
    framed: bool,
    spine_chars: Vec<u32>,
    pub tips: Vec<Tip>,
}

impl Engine {
    /// Build the growth plan from the config and the tree window. Returns
    /// None for a degenerate window (nothing can grow, like cbonsai's NULL
    /// tree window).
    pub fn new(conf: &Config, win: Option<&Window>, rng: &mut GlibcRand) -> Option<Engine> {
        let win = win?;
        let win_w = win.w;
        let win_h = win.h;
        if win_w < 3 || win_h < 2 {
            return None;
        }
        let y_start = win_h - 1;
        let x_center0 = conf.base_origin_x + conf.base_width / 2;
        let life = conf.life_start.clamp(4, 200);
        let room = (y_start - CEIL_Y).max(1);
        let trunk_rows = life.min(room).max(1);

        // The trunk stays inside the pot's footprint when there is one.
        let pot_cap = if conf.base_width > 4 { conf.base_width - 2 } else { win_w - 2 };
        let w_start = (6 + life / 4).clamp(8, 15).min(pot_cap).max(4);
        let w_max = (w_start + 2).min(pot_cap);
        let w_min = (w_start - 3).max(4);
        let trunk_l = x_center0 - w_start / 2;

        let framed = rng.rand() % 4 != 0;
        let max_arms = (1 + conf.multiplier / 2).clamp(1, 6);
        let spawn_floor = y_start - (trunk_rows / 4).max(2);
        // Plan the arms: at least one, spread over the upper three quarters
        // of the trunk with jitter, sides mostly alternating. No arms in the
        // lowest quarter (real saguaros branch well above the pot), none in
        // the crown zone, and room for each arm to top out below it.
        let n_arms = 1 + (rng.rand() as usize % max_arms as usize);
        let mut arm_plan: Vec<(i32, i32)> = Vec::new();
        let hi = spawn_floor;
        let lo = (CEIL_Y + 8).min(hi);
        if hi >= lo {
            let span = (hi - lo) as usize;
            let mut last: Option<(i32, i32)> = None; // (row, side)
            for i in 0..n_arms {
                let side = if i > 0 && rng.rand() % 10 < 7 {
                    -last.map(|(_, s)| s).unwrap_or(1)
                } else if rng.rand() % 2 == 0 {
                    -1
                } else {
                    1
                };
                let base = hi - (span * i / n_arms.max(2)) as i32;
                let slack = ((span / n_arms.max(2)) as i32).min(2);
                let jitter = if slack > 0 { rng.rand() % (slack + 1) as i32 } else { 0 };
                let mut row = (base - jitter).clamp(lo, hi);
                if let Some((lr, ls)) = last {
                    if lr == row && ls == side {
                        row -= 1; // nudge same-side arms off shared rows
                    }
                }
                last = Some((row, side));
                arm_plan.push((row, side));
            }
        }

        let trunk = Column {
            trunk: true,
            side: 0,
            l: trunk_l,
            w: w_start,
            y: y_start,
            rows_left: trunk_rows,
            total_rows: trunk_rows,
            drift_rows: 0,
            drift_every: 1,
            drift_tick: 0,
            w_min,
            w_max,
            started: false,
            prev_l: trunk_l,
            prev_w: w_start,
        };
        Some(Engine {
            queue: vec![trunk],
            occ: vec![false; (win_w.max(0) as usize) * (win_h.max(0) as usize)],
            win_w,
            win_h,
            x_center0,
            arm_plan,
            next_arm: 0,
            framed,
            spine_chars: spine_chars(conf),
            tips: Vec::new(),
        })
    }

    /// One animation step: draw one slice of one column (round-robin).
    pub fn step(&mut self, ctx: &mut Ctx) -> StepOutcome {
        if self.queue.is_empty() {
            return StepOutcome::Done;
        }
        let mut col = self.queue.remove(0);
        let alive = if col.trunk { self.trunk_step(ctx, &mut col) } else { self.arm_step(ctx, &mut col) };
        if alive {
            self.queue.push(col);
        }
        StepOutcome::Drew
    }

    fn verbose(&self, ctx: &mut Ctx, col: &Column) {
        if ctx.conf.verbosity > 0 {
            ctx.tree_print(4, 5, format!("row: {}", crate::output::fmt_02d(col.y)).as_bytes());
            ctx.tree_print(5, 5, format!("width: {}", crate::output::fmt_02d(col.w)).as_bytes());
            ctx.tree_print(6, 5, format!("arms: {}", crate::output::fmt_02d(self.next_arm as i32)).as_bytes());
            ctx.tree_print(7, 5, format!("columns: {}", crate::output::fmt_02d(self.queue.len() as i32)).as_bytes());
        }
    }

    fn trunk_step(&mut self, ctx: &mut Ctx, col: &mut Column) -> bool {
        if col.started {
            col.y -= 1;
            let rng = &mut ctx.rng;
            let progress =
                (col.total_rows - col.rows_left) as f64 / col.total_rows.max(1) as f64;
            // width: bulge low, taper high, wander in between
            let roll = rng.rand() % 10;
            if progress < 0.2 && col.w < col.w_max && roll < 4 {
                col.w += 1;
            } else if progress > 0.72 && col.w > col.w_min && roll < 5 {
                col.w -= 1;
            } else if roll < 2 && col.w > col.w_min {
                col.w -= 1;
            } else if roll < 4 && col.w < col.w_max {
                col.w += 1;
            }
            // lateral sway, mean-reverted around the pot center
            if rng.rand() % 10 < 3 {
                let center = col.l + col.w / 2;
                let mut dir = if rng.rand() % 2 == 0 { -1 } else { 1 };
                if center > self.x_center0 + 2 {
                    dir = -1;
                }
                if center < self.x_center0 - 2 {
                    dir = 1;
                }
                col.l += dir;
            }
            self.spawn_planned(ctx, col);
        }
        col.started = true;
        self.verbose(ctx, col);
        self.draw_slice(ctx, col);
        col.rows_left -= 1;
        if col.rows_left <= 0 || col.y - 1 < CEIL_Y {
            self.draw_cap(ctx, col);
            return false;
        }
        true
    }

    fn arm_step(&mut self, ctx: &mut Ctx, col: &mut Column) -> bool {
        if col.started {
            col.y -= 1;
            if col.drift_rows > 0 {
                col.drift_tick += 1;
                if col.drift_tick % col.drift_every == 0 {
                    col.l += col.side;
                    col.drift_rows -= 1;
                }
            }
        }
        col.started = true;
        self.verbose(ctx, col);
        // swallowed by another column? merge silently, no cap, no bloom
        if self.row_fully_occupied(col.y, col.l, col.w) {
            return false;
        }
        self.draw_slice(ctx, col);
        col.rows_left -= 1;
        if col.rows_left <= 0 || col.y - 1 < CEIL_Y {
            self.draw_cap(ctx, col);
            return false;
        }
        true
    }

    /// Spawn the next planned arm once the trunk reaches its sprout row.
    /// One cell of overlap with the trunk welds the arm on; flavor rolls
    /// (steep vs shallow ramp, width, height, drift) come from the stream.
    fn spawn_planned(&mut self, ctx: &mut Ctx, col: &mut Column) {
        if self.next_arm >= self.arm_plan.len() {
            return;
        }
        let (row, side) = self.arm_plan[self.next_arm];
        if col.y > row {
            return; // not there yet
        }
        // the plan may have been overtaken by an early ceiling cap
        if col.rows_left < 3 || col.y < CEIL_Y + 5 {
            self.next_arm += 1;
            return;
        }
        self.next_arm += 1;
        let rng = &mut ctx.rng;
        let shallow = rng.rand() % 2 == 0;
        let arm_w = (5 + rng.rand() % 3).min(col.w - 1).max(3);
        // arms must top out below the crown zone, never at the ceiling
        let room = (col.y - (CEIL_Y + 4)).max(3);
        let arm_rows = if shallow { 6 + rng.rand() % 8 } else { 5 + rng.rand() % 6 };
        let arm_rows = arm_rows.clamp(3, room);
        let (drift_rows, drift_every) =
            if shallow { (5 + rng.rand() % 4, 2) } else { (3 + rng.rand() % 3, 1) };
        // one cell of overlap with the trunk welds the arm on
        let l = if side > 0 { col.l + col.w - 1 } else { col.l - (arm_w - 1) };
        self.queue.push(Column {
            trunk: false,
            side,
            l,
            w: arm_w,
            y: col.y,
            rows_left: arm_rows,
            total_rows: arm_rows,
            drift_rows,
            drift_every,
            drift_tick: 0,
            w_min: 3,
            w_max: arm_w,
            started: false,
            prev_l: l,
            prev_w: arm_w,
        });
    }

    /// Draw one horizontal slice of body, then sprinkle spines. All RNG
    /// rolls happen before any drawing so the stream order never depends on
    /// draw outcomes.
    fn draw_slice(&mut self, ctx: &mut Ctx, col: &mut Column) {
        let light = ctx.rng.rand() % 2 == 0;
        let (y, l, w) = (col.y, col.l, col.w);
        let pair = if light { PAIR_LIGHT_BODY } else { PAIR_DARK_BODY };
        let bold = light;
        let over_l = col.started && l < col.prev_l;
        let over_r = col.started && (l + w - 1) > (col.prev_l + col.prev_w - 1);
        let framed_row = self.framed && w >= 7;
        let mut cells: Vec<(u8, bool, u8)> = Vec::with_capacity(w.max(0) as usize);
        for i in 0..w {
            let cp = if framed_row && (i == 0 || i == w - 1) {
                b'.'
            } else if framed_row && (i == 1 || i == w - 2) {
                b'_'
            } else if (over_l && i == 0) || (over_r && i == w - 1) {
                b'.'
            } else if (over_l && i == 1) || (over_r && i == w - 2) {
                b'_'
            } else {
                let edge = !framed_row && (i == 0 || i == w - 1);
                tex_roll(&mut ctx.rng, if edge { EDGE_TEX } else { BODY_TEX })
            };
            cells.push((cp, bold, pair));
        }
        for (i, &(cp, b, p)) in cells.iter().enumerate() {
            let x = l + i as i32;
            ctx.put_cell(y, x, cp as u32, b, p);
            self.occ_set(y, x);
        }
        self.spines(ctx, l, w, y);
        col.prev_l = l;
        col.prev_w = w;
    }

    /// Spines stick out from the silhouette into empty cells.
    fn spines(&mut self, ctx: &mut Ctx, l: i32, w: i32, y: i32) {
        if self.spine_chars.is_empty() {
            return;
        }
        let roll_l = ctx.rng.rand() % 100;
        let roll_r = ctx.rng.rand() % 100;
        let mut placed: Vec<(i32, u32, bool, u8)> = Vec::new();
        for (roll, x) in [(roll_l, l - 1), (roll_r, l + w)] {
            if roll >= 30 {
                continue;
            }
            if x < 0 || x >= self.win_w || self.occ_get(y, x) {
                continue;
            }
            let cp = self.spine_chars[(ctx.rng.rand() as usize) % self.spine_chars.len()];
            let light = ctx.rng.rand() % 10 < 4;
            let pair = if light { PAIR_LIGHT_SPINE } else { PAIR_DARK_SPINE };
            placed.push((x, cp, light, pair));
        }
        for (x, cp, light, pair) in placed {
            ctx.put_cell(y, x, cp, light, pair);
        }
    }

    /// Close a finished column with its shoulder cap and record the tip.
    fn draw_cap(&mut self, ctx: &mut Ctx, col: &Column) {
        let light = ctx.rng.rand() % 2 == 0;
        let pair = if light { PAIR_LIGHT_BODY } else { PAIR_DARK_BODY };
        let y = col.y - 1;
        let (l, w) = (col.l, col.w);
        if y < 0 {
            return;
        }
        if !col.trunk && self.row_fully_occupied(y, l, w) {
            return; // merged into a neighbor: no cap, no bloom
        }
        let mut chars: Vec<u8> = Vec::with_capacity(w.max(0) as usize);
        for i in 0..w {
            chars.push(if w >= 7 {
                match i {
                    0 => b'\\',
                    1 => b'd',
                    _ if i == w - 2 => b'b',
                    _ if i == w - 1 => b'/',
                    _ => tex_roll(&mut ctx.rng, BODY_TEX),
                }
            } else {
                match i {
                    0 => b'\\',
                    _ if i == w - 1 => b'/',
                    _ => tex_roll(&mut ctx.rng, BODY_TEX),
                }
            });
        }
        for (i, &cp) in chars.iter().enumerate() {
            let x = l + i as i32;
            if y < self.win_h && x >= 0 && x < self.win_w {
                ctx.put_cell(y, x, cp as u32, true, pair);
                self.occ_set(y, x);
            }
        }
        self.tips.push(Tip { y: y - 1, x: l + w / 2, crown: col.trunk });
    }

    fn occ_idx(&self, y: i32, x: i32) -> Option<usize> {
        if y < 0 || x < 0 || y >= self.win_h || x >= self.win_w {
            return None;
        }
        Some((y * self.win_w + x) as usize)
    }

    fn occ_get(&self, y: i32, x: i32) -> bool {
        self.occ_idx(y, x).map(|i| self.occ[i]).unwrap_or(true)
    }

    fn occ_set(&mut self, y: i32, x: i32) {
        if let Some(i) = self.occ_idx(y, x) {
            self.occ[i] = true;
        }
    }

    fn row_fully_occupied(&self, y: i32, l: i32, w: i32) -> bool {
        (0..w).all(|i| self.occ_get(y, l + i))
    }
}

#[cfg(test)]
mod visual {
    use super::*;
    use crate::config::Config;
    use crate::ctx::Ctx;
    use crate::output::grid_text;

    fn render(seed: i32, rows: i32, cols: i32, mult: i32) -> String {
        let mut conf = Config::new();
        conf.seed = seed;
        conf.multiplier = mult;
        let mut ctx = Ctx::new_headless_sized(conf, rows, cols);
        let r = ctx.grow_cactus_test(0);
        assert!(matches!(r, GrowthResult::Complete));
        let snapshot = ctx.compose_snapshot();
        grid_text(&snapshot, ctx.screen.rows, ctx.screen.cols)
    }

    #[test]
    fn dump_seeds_for_visual_review() {
        for seed in 1..=4 {
            println!("===== seed {seed} =====");
            println!("{}", render(seed, 26, 64, 5));
        }
        println!("===== base 2 / seed 11 =====");
        {
            let mut conf = Config::new();
            conf.seed = 11;
            conf.base_type = 2;
            let mut ctx = Ctx::new_headless_sized(conf, 30, 70);
            let _ = ctx.grow_cactus_test(0);
            println!("{}", grid_text(&ctx.compose_snapshot(), 30, 70));
        }
        println!("===== -M 20 / seed 12 =====");
        println!("{}", render(12, 32, 70, 20));
        println!("===== -M 1 -L 12 / seed 13 =====");
        {
            let mut conf = Config::new();
            conf.seed = 13;
            conf.multiplier = 1;
            conf.life_start = 12;
            let mut ctx = Ctx::new_headless_sized(conf, 24, 64);
            let _ = ctx.grow_cactus_test(0);
            println!("{}", grid_text(&ctx.compose_snapshot(), 24, 64));
        }
        println!("===== -L 200 / seed 14 (tall) =====");
        println!("{}", render(14, 42, 70, 5));
        println!("===== no pot -b 0 / seed 15 =====");
        {
            let mut conf = Config::new();
            conf.seed = 15;
            conf.base_type = 0;
            let mut ctx = Ctx::new_headless_sized(conf, 24, 64);
            let _ = ctx.grow_cactus_test(0);
            println!("{}", grid_text(&ctx.compose_snapshot(), 24, 64));
        }
    }
}
