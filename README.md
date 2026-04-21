# libghostty-rs

Rust bindings and safe API for [libghostty-vt](https://ghostty.org), the virtual terminal emulator library extracted from [Ghostty](https://ghostty.org).

## Workspace Layout

- `crates/libghostty-vt-sys` — raw FFI bindings generated from `ghostty/vt.h`
- `crates/libghostty-vt` — safe Rust wrappers (Terminal, RenderState, KeyEncoder, MouseEncoder, etc.)
- `example/ghostling_rs` — Rust port of [ghostling](https://github.com/ghostty-org/ghostling), a minimal terminal emulator using [macroquad](https://macroquad.rs)

## Quick Start

```rust
use libghostty_vt::{Terminal, TerminalOptions, RenderState};
use libghostty_vt::render::{RowIterator, CellIterator};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a terminal with 80 columns, 24 rows, and scrollback.
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 80,
        rows: 24,
        max_scrollback: 10_000,
    })?;

    // Register an effect handler for PTY write-back (e.g. query responses).
    terminal.on_pty_write(|_term, data| {
        println!("PTY response: {} bytes", data.len());
    })?;

    // Feed VT-encoded data into the terminal.
    terminal.vt_write(b"Hello, \x1b[1;32mworld\x1b[0m!\r\n");
    terminal.vt_write(b"\x1b[38;2;255;128;0morange text\x1b[0m\r\n");

    // Capture a render snapshot and iterate rows/cells.
    let mut render_state = RenderState::new()?;
    let mut rows = RowIterator::new()?;
    let mut cells = CellIterator::new()?;

    let snapshot = render_state.update(&terminal)?;
    let mut row_iter = rows.update(&snapshot)?;

    while let Some(row) = row_iter.next() {
        let mut cell_iter = cells.update(row)?;
        while let Some(cell) = cell_iter.next() {
            let graphemes: Vec<char> = cell.graphemes()?;
            print!("{graphemes:?}");
        }
        println!();
    }

    Ok(())
}
```

## Building

Requires [Zig](https://ziglang.org/) 0.15.x on PATH. By default, the ghostty
source is fetched automatically at build time from the pinned commit in
`build.rs`. Set `GHOSTTY_SOURCE_DIR` to make the build use a local Ghostty
checkout instead.

The `pkg-config` path is opt-in. If you enable `libghostty-vt-sys/pkg-config`,
the build will prefer an installed `libghostty-vt` discovered through
`pkg-config` only when `GHOSTTY_SOURCE_DIR` is unset. An explicit
`GHOSTTY_SOURCE_DIR` always wins.

```sh
nix develop
cargo check
cargo test -p libghostty-vt-sys
cargo build -p ghostling_rs
```

### Running the example

```sh
# Linux
LD_LIBRARY_PATH=$(dirname $(find target/debug/build/libghostty-vt-sys-*/out -name "libghostty-vt*" | head -1)) \
  cargo run -p ghostling_rs

# macOS
DYLD_LIBRARY_PATH=$(dirname $(find target/debug/build/libghostty-vt-sys-*/out -name "libghostty-vt*" | head -1)) \
  cargo run -p ghostling_rs
```
