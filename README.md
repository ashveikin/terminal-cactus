# tcac

A beautifully random cactus generator — the rbonsai engine rebuilt around a
cactus. Dependency-free like its ancestor: no ncurses, no crates at all. The
terminal is handled with hand-rolled ANSI + termios, the CLI parser is a GNU
getopt_long replica, and the growth RNG is a glibc `rand()` clone pinned by
golden tests, so **every seed grows the same cactus every time**.

## Build & run

```sh
bin/build            # cargo build --release
./target/release/tcac -l          # live growth
./target/release/tcac -S          # screensaver (quit on any keypress)
./target/release/tcac -p -s 42    # print cactus for seed 42
./target/release/tcac --help
```

## What it grows

A saguaro-style cactus: a ribbed trunk rising out of a terracotta pot, arms
that sprout from the trunk sides, lean outward diagonally, then straighten
into their own columns, each column closed with a shoulder cap (`\d...b/`).
Two silhouettes per seed: framed slices (`._|||_.`, reference art 2) or
plain textured rims (reference art 1).

When the last column tops out, the **bloom finale** runs: buds open into
flowers tip by tip, top to crown, in pink, red, yellow and white — and the
trunk crown sometimes carries a whole `|o|o|` band, like a real cactus
putting out its season's bloom. Spines stick out of the silhouette; `-c`
picks their characters (botanically, cactus spines are modified leaves).

Every seed generates a new plant: arm schedule, widths, sway, texture and
bloom all come from the one RNG stream in a fixed order, so live mode, `-p`
print, save/load and resize replay all render the identical cactus for a
given seed.

## CLI

Every rbonsai/cbonsai option works the same way:

| option | meaning |
|---|---|
| `-l, --live` | live mode: show each step of growth |
| `-t, --time=TIME` | seconds between growth steps [default: 0.03] |
| `-i, --infinite` | keep growing cacti (a new seed each time) |
| `-w, --wait=TIME` | seconds between infinite generations [default: 4] |
| `-S, --screensaver` | `-li` plus quit on any keypress |
| `-m, --message=STR` | attach a message box next to the cactus |
| `-T, --text-origin=ROW,COL` | display text from stdin at a position |
| `-g, --geometry=ROW,COL` | place the pot yourself |
| `-b, --base=INT` | ascii-art pot, 0 = none (1 straight, 2 tapered) |
| `-c, --leaf=LIST` | comma-delimited spine characters [default: '] |
| `-k, --color=LIST` | 4 color indices: dark body, dark spines, light body, light spines [default: 2,3,10,11] |
| `-M, --multiplier=INT` | arm multiplier; higher -> more arms (0-20) |
| `-L, --life=INT` | life; higher -> more growth (0-200) |
| `-p, --print` | print the cactus when finished |
| `-s, --seed=INT` | seed the RNG |
| `-W, --save=FILE` / `-C, --load=FILE` | save/load growth progress |
| `-v, --verbose` / `-h, --help` | debug overlay / help |

Progress saves as `seed growth-steps` in the file (default
`$XDG_CACHE_HOME/tcac` or `~/.cache/tcac`); loading replays the same plant
to the same point, mid-bloom included. Ctrl-C tears the terminal down the
same way cbonsai does; `reset` fixes it.

## Determinism contracts

- `src/rand.rs` clones glibc's TYPE_3 `rand()`; golden sequences from the
  host libc live in `tools/rand_golden.txt` and are asserted by `cargo test`.
- `LINES`/`COLUMNS` env vars override the terminal size, like ncurses.
- All random decisions are rolled before the pixels they touch move, so the
  stream never depends on rendering outcomes.

## Layout

```
src/main.rs          entry: arg parsing + wiring
src/config.rs        struct config, getopt consumer, sanitized CLI errors
src/getopt.rs        GNU getopt_long replica (bundling, abbreviation, permutation)
src/numparse.rs      strtod/strtol with C semantics
src/rand.rs          glibc rand()/srand() clone + cvttsd2si int casts
src/utf8.rs          mbrtowc-semantics decoder
src/wcwidth.rs       glibc-compatible wcwidth/iswspace/iswcntrl
src/wcwidth_table.rs generated table
src/screen.rs        cells, windows, panel merge, overlay/overwrite, stdscr layer
src/terminal.rs      termios raw mode, winsize, SIGWINCH, input, ANSI output
src/message.rs       layoutMessage word-wrap port
src/cactus.rs        the growth engine: trunk, arms, slices, spines, caps
src/bloom.rs         the flower finale: shapes, palette, bud animation
src/output.rs        pair->SGR tables, printstdscr, plain-text frame dumps
src/save.rs          save/load files
src/ctx.rs           application flow: init, growth, bloom, modes
```

## Tests & visual review

```sh
bin/test                          # cargo test + pty smoke tests
tools/render_seeds.sh 1 12        # 12 seeds as plain text for eyeballing
cargo test write_golden_snapshot -- --ignored   # refresh the golden art
```

`cargo test` covers the RNG goldens, CLI parsing (including the sanitized
error paths), message layout, determinism (same seed -> identical grid),
save/load round-trips, resize replay and a golden snapshot of seed 42's
rendered art. `tools/tests/pty_smoke.py` drives live, static, screensaver,
infinite, print and resize behavior under a real pty.
