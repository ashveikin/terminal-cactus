//! The application state and flow — rbonsai's ncursesObjects/config/
//! counters structure adapted to the cactus: init (curses setup, colors,
//! pot geometry, message, -T text), growth via the cactus engine, the bloom
//! finale, then infinite/screensaver/print/static modes.

use crate::bloom;
use crate::cactus::{self, Engine, GrowthResult};
use crate::config::{Config, InputResult};
use crate::output;
use crate::rand::GlibcRand;
use crate::save;
use crate::screen::{Screen, Window};
use crate::terminal::Terminal;
use crate::{message, terminal};

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERR: i32 = 1;

pub struct Counters {
    /// Slices drawn + bloom frames; the save/load replay position.
    pub growth_steps: i32,
    pub replay_until: i32,
}

pub struct Ctx {
    pub conf: Config,
    pub screen: Screen,
    pub term: Terminal,
    pub base_win: Option<Window>,
    pub tree_win: Option<Window>,
    pub message_border_win: Option<Window>,
    pub message_win: Option<Window>,
    pub stdout_out: Vec<u8>,
    pub stderr_out: Vec<u8>,
    pub rng: GlibcRand,
    pub counters: Counters,
    pub term_ready: bool,
}

impl Ctx {
    pub fn new(conf: Config, stdout_out: Vec<u8>, stderr_out: Vec<u8>) -> Ctx {
        let (rows, cols) = terminal::terminal_size();
        let rng = GlibcRand::srand(conf.seed as u32);
        let colors = conf.colors;
        let mut term = Terminal::new();
        term.user_colors = colors;
        Ctx {
            screen: Screen::new(rows, cols),
            term,
            base_win: None,
            tree_win: None,
            message_border_win: None,
            message_win: None,
            stdout_out,
            stderr_out,
            rng,
            counters: Counters { growth_steps: 0, replay_until: 0 },
            term_ready: false,
            conf,
        }
    }

    #[cfg(test)]
    pub fn new_headless_sized(mut conf: Config, rows: i32, cols: i32) -> Ctx {
        conf.headless = true;
        let rng = GlibcRand::srand(conf.seed as u32);
        let colors = conf.colors;
        let mut term = Terminal::new();
        term.user_colors = colors;
        let mut ctx = Ctx {
            screen: Screen::new(rows, cols),
            term,
            base_win: None,
            tree_win: None,
            message_border_win: None,
            message_win: None,
            stdout_out: Vec::new(),
            stderr_out: Vec::new(),
            rng,
            counters: Counters { growth_steps: 0, replay_until: 0 },
            term_ready: false,
            conf,
        };
        ctx.init(false);
        ctx
    }

    pub fn flush_and_exit(&mut self, code: i32) -> ! {
        use std::io::Write;
        let _ = std::io::stdout().write_all(&self.stdout_out);
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().write_all(&self.stderr_out);
        let _ = std::io::stderr().flush();
        std::process::exit(code);
    }

    /// `update_panels(); doupdate();` — composite and render.
    fn update_panels_doupdate(&mut self) {
        let (rows, cols) = (self.screen.rows, self.screen.cols);
        let snapshot = self.compose_snapshot();
        if self.term_ready {
            self.term.render(rows, cols, &snapshot);
        }
    }

    /// Composite the window stack (base, tree, message border, message).
    pub fn compose_snapshot(&mut self) -> Vec<crate::screen::Cell> {
        let mut panels: Vec<&Window> = Vec::new();
        if let Some(b) = &self.base_win {
            panels.push(b);
        }
        if let Some(t) = &self.tree_win {
            panels.push(t);
        }
        if let Some(b) = &self.message_border_win {
            panels.push(b);
        }
        if let Some(m) = &self.message_win {
            panels.push(m);
        }
        self.screen.update_panels_doupdate(&panels)
    }

    /// `updateScreen` (display changes + sleep), scaled for bloom pacing.
    fn update_screen_scaled(&mut self, scale: f32) {
        self.update_panels_doupdate();
        if self.term_ready {
            let step = self.conf.time_step * scale;
            let secs = (step / 1.0) as u64;
            let nsecs = ((step - secs as f32) * 1_000_000_000.0) as u32;
            std::thread::sleep(std::time::Duration::new(secs, nsecs));
        }
    }

    fn maybe_update_screen(&mut self, scale: f32) {
        if self.conf.live
            && self.counters.growth_steps >= self.counters.replay_until
            && !(self.conf.load && self.counters.growth_steps < self.conf.target_steps)
        {
            self.update_screen_scaled(scale);
        }
    }

    /// mvwprintw(treeWin, y, x, ...) with ncurses clipping; NULL window
    /// (degenerate geometry) makes every draw a no-op.
    pub fn tree_print(&mut self, y: i32, x: i32, text: &[u8]) {
        let Some(win) = &mut self.tree_win else { return };
        if !win.wmove(y, x) {
            return;
        }
        win.addstr_bytes(text);
    }

    /// One attribute-tagged cell into the tree window (the engine's draw
    /// primitive).
    pub fn put_cell(&mut self, y: i32, x: i32, cp: u32, bold: bool, pair: u8) {
        let Some(win) = &mut self.tree_win else { return };
        if !win.wmove(y, x) {
            return;
        }
        win.attr_on(Some(bold), Some(pair));
        win.addch(cp);
    }

    /// init(): curses setup, colors, geometry, windows, message, -T text.
    pub fn init(&mut self, resizing: bool) {
        if resizing {
            // resizeterm(0, 0); erase();
            let (rows, cols) = terminal::terminal_size();
            if rows != self.screen.rows || cols != self.screen.cols {
                self.screen.resize(rows, cols);
            }
            self.screen.stdscr_erase();
            if !self.conf.headless {
                self.term_ready = true;
                self.term.init();
            }
        } else {
            if !self.conf.headless {
                self.term_ready = true;
                self.term.init();
            }
            self.init_colors();
        }

        self.set_base_geometry();
        if !resizing {
            self.conf.growth_height = self.conf.base_origin_y;
        }

        self.draw_wins();
        self.draw_message();

        // -T ROW,COL: text into stdscr (visible at refresh) and into the
        // tree window (visible via panels).
        if self.conf.text_origin_y >= 0 && self.conf.text_origin_x >= 0 {
            if let Some(text) = self.conf.text.clone() {
                let (y, x) = (self.conf.text_origin_y, self.conf.text_origin_x);
                self.stdscr_print_at(y, x, &text);
                if let Some(win) = &mut self.tree_win {
                    win.wmove(y, x);
                    win.addstr_bytes(&text);
                }
            }
        }
    }

    /// mvprintw on stdscr with touched-line tracking.
    fn stdscr_print_at(&mut self, y: i32, x: i32, bytes: &[u8]) {
        if !self.screen.stdscr.wmove(y, x) {
            return;
        }
        self.screen.stdscr.addstr_bytes(bytes);
        let end_row = self.screen.stdscr.cy.max(y);
        for row in y..=end_row {
            self.screen.stdscr_write_mark(row);
        }
    }

    /// The has_colors/start_color block with its 8-color warning path.
    fn init_colors(&mut self) {
        if self.term.colors > 0 {
            let mut warned_already = false;
            for id in 1..=4usize {
                let color_id = self.conf.colors[id - 1];
                if !warned_already && self.term.colors < 256 && color_id >= 8 {
                    if self.term_ready {
                        self.term.endwin();
                        self.term_ready = false;
                    }
                    self.stdout_out
                        .extend_from_slice(b"Warning: defaulting to 8-color support.\n");
                    warned_already = true;
                    self.update_panels_doupdate();
                }
            }
        } else {
            self.stdout_out
                .extend_from_slice(b"Warning: terminal does not have color support.\n");
        }
    }

    fn set_base_geometry(&mut self) {
        match self.conf.base_type {
            1 => {
                self.conf.base_width = 13;
                self.conf.base_height = 6;
            }
            2 => {
                self.conf.base_width = 19;
                self.conf.base_height = 8;
            }
            _ => {
                self.conf.base_width = 0;
                self.conf.base_height = 0;
            }
        }
        let rows = self.screen.rows;
        let cols = self.screen.cols;
        if self.conf.automatic_base_position {
            self.conf.base_origin_y = rows - self.conf.base_height;
            self.conf.base_origin_x = (cols / 2) - (self.conf.base_width / 2);
        }
    }

    fn draw_wins(&mut self) {
        let (rows, cols) = (self.screen.rows, self.screen.cols);
        self.base_win = Window::newwin(
            self.conf.base_height,
            self.conf.base_width,
            self.conf.base_origin_y,
            self.conf.base_origin_x,
            rows,
            cols,
        );
        self.tree_win = Window::newwin(self.conf.base_origin_y, cols, 0, 0, rows, cols);
        self.message_border_win = None;
        self.message_win = None;
        self.draw_pot();
    }

    /// drawPot: the two ascii-art pots from the reference art, in
    /// terracotta; skipped entirely when newwin returned NULL.
    fn draw_pot(&mut self) {
        let Some(pot) = &mut self.base_win else { return };
        let pot_pair = output::PAIR_POT;
        let row = |pot: &mut Window, y: i32, x: i32, s: &[u8], bold: bool| {
            pot.attr_on(Some(bold), Some(pot_pair));
            pot.wmove(y, x);
            pot.addstr_bytes(s);
        };
        match self.conf.base_type {
            1 => {
                row(pot, 0, 0, b".===========.", true);
                row(pot, 1, 0, b"[___________]", false);
                row(pot, 2, 1, b"|         |", false);
                row(pot, 3, 1, b"|         |", false);
                row(pot, 4, 1, b"|         |", false);
                row(pot, 5, 1, b"|_________|", false);
            }
            2 => {
                row(pot, 0, 0, b".=================.", true);
                row(pot, 1, 0, b"|_________________|", false);
                row(pot, 2, 1, b"|               |", false);
                row(pot, 3, 2, b"|             |", false);
                row(pot, 4, 2, b"|             |", false);
                row(pot, 5, 3, b"|           |", false);
                row(pot, 6, 3, b"|           |", false);
                row(pot, 7, 4, b"|_________|", false);
            }
            _ => {}
        }
        pot.attr_off_bold();
    }

    /// drawMessage + createMessageWindows: layout, border box, content.
    fn draw_message(&mut self) {
        let Some(message_bytes) = self.conf.message.clone() else {
            return;
        };

        let (max_y, max_x) = (self.screen.rows, self.screen.cols);
        if max_x < 5 || max_y < 3 {
            self.stderr_out
                .extend_from_slice(b"error: terminal is too small to display the message\n");
            return;
        }

        let mut max_content_width = (max_x - 4) / 4;
        if max_content_width < 1 {
            max_content_width = 1;
        }

        let layout = message::layout_message(&message_bytes, max_content_width);

        let content_width = if layout.width != 0 { layout.width } else { 1 };
        let content_height = std::cmp::min(layout.lines.len() as i32, max_y - 2);

        // createMessageWindows: a border needs two columns per side and two
        // rows; refuse when the terminal cannot contain one.
        if max_x < 5 || max_y < 3 {
            self.stderr_out
                .extend_from_slice(b"error: terminal is too small to display the message\n");
            return;
        }
        let border_width = content_width + 4;
        let border_height = content_height + 2;
        let mut border_y = (max_y * 7) / 10 - 1;
        let mut border_x = (max_x * 7) / 10 - 2;
        if border_y < 0 {
            border_y = 0;
        }
        if border_x < 0 {
            border_x = 0;
        }
        if border_y + border_height > max_y {
            border_y = max_y - border_height;
        }
        if border_x + border_width > max_x {
            border_x = max_x - border_width;
        }

        let mut border_win = Window::new(border_height, border_width, border_y, border_x);
        let mut message_win = Window::new(content_height, content_width, border_y + 1, border_x + 2);

        // Draw the box before exposing the windows (COLOR_TEXT | A_BOLD).
        border_win.attr_on(Some(true), Some(output::PAIR_TEXT));
        border_win.border();

        for y in 0..content_height {
            message_win.wmove(y, 0);
            message_win.addwstr(&layout.lines[y as usize]);
        }
        // make clipping visible instead of silently dropping lines
        if (layout.lines.len() as i32) > content_height {
            let marker: &[u8] = if content_width >= 3 { b"..." } else { b"." };
            let marker_x = content_width - marker.len() as i32;
            message_win.wmove(content_height - 1, marker_x);
            message_win.addstr_bytes(marker);
        }

        if self.conf.verbosity >= 2 {
            let text = format!("message lines: {}", layout.lines.len());
            if let Some(win) = &mut self.tree_win {
                win.wmove(11, 5);
                win.addstr_bytes(text.as_bytes());
            }
        }

        self.message_border_win = Some(border_win);
        self.message_win = Some(message_win);
    }

    /// checkKeyPress: polled input during growth (on stdscr).
    pub fn check_key_press(&mut self) -> InputResult {
        // wgetch(stdscr) internally refreshes a modified window first
        self.screen.flush_touched();
        if terminal::take_terminal_resize() {
            return InputResult::Resize;
        }
        let key = if self.conf.headless { None } else { self.term.getch_nodelay().flatten() };
        self.classify_key(key)
    }

    /// checkKeyPress with a getch timeout (infinite mode's wait).
    fn check_key_press_timeout(&mut self, timeout_ms: i32) -> InputResult {
        self.screen.flush_touched();
        if terminal::take_terminal_resize() {
            return InputResult::Resize;
        }
        let key = if self.conf.headless {
            None
        } else {
            self.term.getch_timeout(timeout_ms).flatten()
        };
        self.classify_key(key)
    }

    fn classify_key(&mut self, key: Option<u8>) -> InputResult {
        if terminal::take_terminal_resize() {
            return InputResult::Resize;
        }
        match key {
            Some(k) => {
                if self.conf.screensaver || k == b'q' {
                    InputResult::Quit
                } else {
                    InputResult::Continue
                }
            }
            None => InputResult::Continue,
        }
    }

    /// finish(): refresh frame, endwin, save.
    fn finish(&mut self) {
        let (rows, cols) = (self.screen.rows, self.screen.cols);
        let snapshot = {
            let mut panels: Vec<&Window> = Vec::new();
            if let Some(b) = &self.base_win {
                panels.push(b);
            }
            if let Some(t) = &self.tree_win {
                panels.push(t);
            }
            if let Some(b) = &self.message_border_win {
                panels.push(b);
            }
            if let Some(m) = &self.message_win {
                panels.push(m);
            }
            // refresh(): stdscr's changed lines join the composite here
            self.screen.refresh_snapshot(&panels)
        };
        if self.term_ready {
            self.term.render(rows, cols, &snapshot);
            self.term.endwin();
            self.term_ready = false;
        }
        if self.conf.save {
            let mut out = std::mem::take(&mut self.stdout_out);
            save::save_to_file(
                &self.conf.save_file.clone(),
                self.conf.seed,
                self.counters.growth_steps,
                &mut out,
            );
            self.stdout_out = out;
        }
    }

    /// redrawCurrentCactus: replay the completed growth at the new geometry.
    fn redraw_current_cactus(&mut self) {
        let mut replay_until = self.counters.growth_steps;
        let mut result = GrowthResult::Resize;
        while matches!(result, GrowthResult::Resize) {
            self.rng = GlibcRand::srand(self.conf.seed as u32);
            self.init(true);
            result = self.grow_cactus(replay_until);
            replay_until = self.counters.growth_steps;
        }
    }

    #[cfg(test)]
    pub fn grow_cactus_test(&mut self, replay_until: i32) -> GrowthResult {
        self.grow_cactus(replay_until)
    }

    fn grow_cactus(&mut self, replay_until: i32) -> GrowthResult {
        let (max_y, max_x) = match &self.tree_win {
            Some(w) => (w.h, w.w),
            None => (0, 0),
        };

        self.counters.growth_steps = 0;
        self.counters.replay_until = replay_until;

        if self.conf.verbosity > 0 {
            let text = format!("maxX: {}, maxY: {}", output::fmt_03d(max_x), output::fmt_03d(max_y));
            self.tree_print(2, 5, text.as_bytes());
        }

        // A degenerate tree window grows nothing (cbonsai's NULL-window path).
        let mut engine = match Engine::new(&self.conf, self.tree_win.as_ref(), &mut self.rng) {
            Some(e) => e,
            None => {
                self.update_panels_doupdate();
                return GrowthResult::Complete;
            }
        };

        // growth: one slice of one column per step, round-robin
        loop {
            match self.check_key_press() {
                InputResult::Quit => return GrowthResult::Quit,
                InputResult::Resize => return GrowthResult::Resize,
                InputResult::Continue => {}
            }
            match engine.step(self) {
                cactus::StepOutcome::Drew => {
                    self.counters.growth_steps += 1;
                    self.maybe_update_screen(1.0);
                }
                cactus::StepOutcome::Done => break,
            }
        }

        // the bloom finale
        let mut bloom_state = bloom::BloomState::new(std::mem::take(&mut engine.tips));
        loop {
            match self.check_key_press() {
                InputResult::Quit => return GrowthResult::Quit,
                InputResult::Resize => return GrowthResult::Resize,
                InputResult::Continue => {}
            }
            match bloom::step(&mut bloom_state, self) {
                bloom::BloomStep::Drew => {
                    self.counters.growth_steps += 1;
                    self.maybe_update_screen(5.0);
                }
                bloom::BloomStep::Done => break,
            }
        }

        self.update_panels_doupdate();
        GrowthResult::Complete
    }

    /// main's do-while loop and the exit paths.
    pub fn run(&mut self) -> ! {
        loop {
            self.init(false);
            match self.grow_cactus(0) {
                GrowthResult::Resize => self.redraw_current_cactus(),
                GrowthResult::Quit => {
                    self.finish();
                    self.flush_and_exit(EXIT_OK);
                }
                GrowthResult::Complete => {}
            }
            if self.conf.load {
                self.conf.target_steps = 0;
            }

            if self.conf.infinite {
                // Keep the completed cactus visible until the timer expires; a
                // resize during the pause redraws and restarts the wait.
                loop {
                    let timeout = crate::rand::c_int_from_f64(self.conf.time_wait * 1000.0);
                    match self.check_key_press_timeout(timeout) {
                        InputResult::Quit => {
                            self.finish();
                            self.flush_and_exit(EXIT_OK);
                        }
                        InputResult::Resize => {
                            self.redraw_current_cactus();
                            continue;
                        }
                        InputResult::Continue => {}
                    }
                    break;
                }
                // Seed the next cactus independently after the current one:
                // every generation is a new plant.
                self.rng = GlibcRand::srand(crate::time_now());
            } else {
                break;
            }
        }

        if self.conf.print_cactus {
            self.finish();

            // overlay all windows onto stdscr
            if let Some(base) = self.base_win.clone() {
                self.screen.overlay(&base);
            }
            if let Some(tree) = self.tree_win.clone() {
                self.screen.overlay(&tree);
            }
            if let Some(mb) = self.message_border_win.clone() {
                self.screen.overwrite(&mb);
            }
            if let Some(m) = self.message_win.clone() {
                self.screen.overwrite(&m);
            }

            let bytes = output::printstdscr(&self.screen, &self.conf.colors, self.term.colors);
            self.stdout_out.extend_from_slice(&bytes);
            self.flush_and_exit(EXIT_OK);
        }

        // Static mode waits for any key, preserving the cactus across resize.
        loop {
            if terminal::take_terminal_resize() {
                self.redraw_current_cactus();
                continue;
            }
            if !self.conf.headless {
                let _key = self.term.getch_blocking();
            }
            if terminal::take_terminal_resize() {
                self.redraw_current_cactus();
                continue;
            }
            self.finish();
            break;
        }
        self.flush_and_exit(EXIT_OK);
    }
}

#[cfg(test)]
mod flow {
    use super::*;
    use crate::output::grid_text;

    fn base_conf(seed: i32) -> Config {
        let mut c = Config::new();
        c.seed = seed;
        c
    }

    fn grow_grid(conf: Config) -> (String, i32) {
        let mut ctx = Ctx::new_headless_sized(conf, 24, 64);
        let _ = ctx.grow_cactus_test(0);
        let text = grid_text(&ctx.compose_snapshot(), ctx.screen.rows, ctx.screen.cols);
        (text, ctx.counters.growth_steps)
    }

    #[test]
    fn same_seed_same_cactus() {
        let (a, _) = grow_grid(base_conf(42));
        let (b, _) = grow_grid(base_conf(42));
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_differ() {
        let (a, _) = grow_grid(base_conf(1));
        let (b, _) = grow_grid(base_conf(2));
        assert_ne!(a, b);
    }

    #[test]
    fn cactus_has_pot_body_and_bloom() {
        let (text, steps) = grow_grid(base_conf(42));
        assert!(text.contains("[___________]"), "pot missing:\n{text}");
        assert!(text.contains('|'), "body missing");
        // flowers never share characters with body texture or caps
        assert!(
            text.contains('@') || text.contains('*') || text.contains('o') || text.contains('('),
            "no bloom:\n{text}"
        );
        // a default-sized plant grows a healthy number of slices and frames
        assert!(steps > 20, "suspiciously few growth steps: {steps}");
    }

    #[test]
    fn save_load_round_trip() {
        let (original, steps) = grow_grid(base_conf(7));
        let path = std::env::temp_dir().join(format!("tcac_test_{}.dat", std::process::id()));
        let mut out = Vec::new();
        assert!(save::save_to_file(path.as_os_str().as_encoded_bytes(), 7, steps, &mut out));

        let mut conf = base_conf(0);
        conf.load = true;
        conf.load_file = path.as_os_str().as_encoded_bytes().to_vec();
        let (seed, count) = save::load_from_file(&conf.load_file, &mut Vec::new()).expect("load");
        conf.seed = seed;
        conf.target_steps = count;
        let (replayed, replayed_steps) = grow_grid(conf);
        assert_eq!(replayed, original);
        assert_eq!(replayed_steps, steps);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn resize_replay_completes() {
        let mut ctx = Ctx::new_headless_sized(base_conf(5), 24, 64);
        let _ = ctx.grow_cactus_test(0);
        let steps = ctx.counters.growth_steps;
        // the redraw path: resizeterm + erase + replay
        ctx.screen.resize(30, 70);
        ctx.init(true);
        let r = ctx.grow_cactus_test(steps);
        assert!(matches!(r, GrowthResult::Complete));
        let text = grid_text(&ctx.compose_snapshot(), ctx.screen.rows, ctx.screen.cols);
        assert!(text.contains('|'), "replayed cactus missing:\n{text}");
    }

    /// Pins the rendered art of seed 42 at 24x64. Regenerate with
    /// `cargo test write_golden_snapshot -- --exact --nocapture` after an
    /// intentional art change, then re-run the suite.
    #[test]
    fn golden_snapshot() {
        let (text, _) = grow_grid(base_conf(42));
        let golden = include_str!("../tools/golden/seed_42.txt");
        assert_eq!(text, golden);
    }

    #[test]
    fn spines_appear() {
        let mut conf = base_conf(3);
        conf.spines = vec![b"\"".to_vec()];
        let (text, _) = grow_grid(conf);
        assert!(text.contains('"'), "no spines drawn:\n{text}");
    }

    #[test]
    fn many_seeds_stay_wellformed() {
        for seed in 20..50 {
            let (text, steps) = grow_grid(base_conf(seed));
            assert!(text.contains("[___________]"), "seed {seed}: pot missing:\n{text}");
            assert!(
                text.contains('@') || text.contains('*') || text.contains('o') || text.contains('('),
                "seed {seed}: no bloom:\n{text}"
            );
            assert!(steps > 20, "seed {seed}: suspiciously few steps: {steps}");
        }
    }

    #[test]
    #[ignore]
    fn write_golden_snapshot() {
        let (text, _) = grow_grid(base_conf(42));
        std::fs::create_dir_all("tools/golden").unwrap();
        std::fs::write("tools/golden/seed_42.txt", text).unwrap();
    }
}
