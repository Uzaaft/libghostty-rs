# libghostty-rs

Rust bindings and safe API for [libghostty-vt](https://ghostty.org), the virtual terminal emulator library extracted from [Ghostty](https://ghostty.org).

## Workspace Layout

- `crates/libghostty-vt-sys` — raw FFI bindings generated from `ghostty/vt.h`
- `crates/libghostty-vt` — safe Rust wrappers (Terminal, RenderState, KeyEncoder, MouseEncoder, etc.)
- `example/grid_ref_tracked_rs` — focused example of tracked grid references following cells through scrollback and reset
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
checkout instead. Package managers that need network-free builds can also set
`GHOSTTY_ZIG_SYSTEM_DIR` to a pre-fetched Zig package directory; this is passed
to `zig build --system` so Zig does not download package dependencies during
the Cargo build script.

Vendored builds derive Zig's optimize mode from Cargo's profile: dev builds use
`Debug`, size-optimized builds use `ReleaseSmall`, and other release builds use
`ReleaseFast`. Set `LIBGHOSTTY_VT_SYS_OPTIMIZE` to `Debug`, `ReleaseSafe`,
`ReleaseFast`, or `ReleaseSmall` to override that choice explicitly.

The `pkg-config` path is opt-in. If you enable `libghostty-vt-sys/pkg-config`,
the build will prefer an installed `libghostty-vt` discovered through
`pkg-config` when `GHOSTTY_SOURCE_DIR` is unset. libghostty-vt is pre-1.0, so
the checked-in bindings are expected to move with the pinned Ghostty source and
do not guarantee compatibility with arbitrary installed C API revisions. An
explicit `GHOSTTY_SOURCE_DIR` always wins.

Nix builds in this repository prefetch the pinned Ghostty source and Ghostty's
Zig package dependencies up front, then set `GHOSTTY_SOURCE_DIR` and
`GHOSTTY_ZIG_SYSTEM_DIR` for the Cargo build. Downstream Nix packaging should
use the same contract rather than adding `git` or allowing network access in
the sandbox.

By default, `libghostty-vt` and `libghostty-vt-sys` link `libghostty-vt.a`.
This statically links the Ghostty VT archive, but the final binary may still
depend on platform runtime libraries. To link the shared library instead,
enable `libghostty-vt/link-dynamic`.

```sh
nix develop
cargo check
cargo test -p libghostty-vt-sys
cargo build -p ghostling_rs
```

## AFL++ fuzzing

`cargo-afl` is the Rust integration for AFL++. The Nix dev shell provides
`cargo-afl` on all supported hosts. AFL++ itself is available in nixpkgs on
Linux, so macOS users should run the checked-in VM and enter the dev shell from
inside Linux:

```sh
nix develop
cargo afl config --build --update
cargo afl build -p libghostty-vt-afl-fuzz

# Linux
LD_LIBRARY_PATH=$(dirname $(find target/debug/build/libghostty-vt-sys-*/out -name "libghostty-vt*" | head -1)) \
  cargo afl fuzz -i fuzz/afl/in -o fuzz/afl/out target/debug/libghostty-vt-afl-fuzz
```

On macOS, boot the Linux VM first and run the Linux commands from `/work`:

```sh
nix run .#afl-vm

# Inside the VM:
cd /work
nix develop
cargo afl config --build --update
cargo afl build -p libghostty-vt-afl-fuzz
LD_LIBRARY_PATH=$(dirname $(find target/debug/build/libghostty-vt-sys-*/out -name "libghostty-vt*" | head -1)) \
  cargo afl fuzz -i fuzz/afl/in -o fuzz/afl/out target/debug/libghostty-vt-afl-fuzz
```

The fuzz target derives terminal dimensions from the first few bytes, feeds the
remaining bytes through `Terminal::vt_write`, resizes the terminal once, and
then walks render-state rows and cells. This exercises the safe Rust API, the
FFI boundary, effect callbacks, VT parsing, resize handling, and rendering
snapshot reads. AFL++ state is written to `fuzz/afl/out`, which is intentionally
not created by default.

To reproduce a crash, pass the crashing input back through the same binary:

```sh
LD_LIBRARY_PATH=$(dirname $(find target/debug/build/libghostty-vt-sys-*/out -name "libghostty-vt*" | head -1)) \
  cargo afl run target/debug/libghostty-vt-afl-fuzz < fuzz/afl/out/default/crashes/id:000000,...
```

### Valgrind sweep over the corpus

`cargo-valgrind` and `valgrind` are wired into the Linux dev shell and the AFL
VM. They are not installed on macOS because upstream valgrind is broken on
`aarch64-darwin`. Use the AFL VM (`nix run .#afl-vm`) on macOS hosts.

`fuzz/afl/valgrind.sh` builds the harness in non-fuzzing mode (it consumes one
input from stdin per process) and runs every file under `fuzz/afl/in/` through
`valgrind --leak-check=full --track-origins=yes`. The harness pulls in
`libghostty-vt` with the `link-static` feature so there is no `LD_LIBRARY_PATH`
dance. Per-input logs land in `target/valgrind-fuzz/`. Only **definite** leaks
fail the run; `still reachable` and `possibly lost` are reported but expected
because Zig's GeneralPurposeAllocator keeps process-lifetime caches and stores
interior pointers in its allocation headers.

```sh
fuzz/afl/valgrind.sh
```

Set `PROFILE=release` to sweep the optimised build and `LOG_DIR=...` to
redirect logs. Set `LIBGHOSTTY_FUZZ_COLS=N` and/or `LIBGHOSTTY_FUZZ_ROWS=N`
(both u16, > 0) to pin the terminal dimensions for stress runs:

```sh
LIBGHOSTTY_FUZZ_COLS=40000 fuzz/afl/valgrind.sh
```

These env vars also work under `cargo afl fuzz`, which makes them useful for
targeting allocation paths that only trigger at large grid sizes.

`LIBGHOSTTY_FUZZ_REPEAT=N` (non-fuzzing build only) loops `fuzz_terminal` N
times in a single process on the same input. Compare the `HEAP SUMMARY` totals
between two runs to detect per-iteration leaks: stable bytes-in-use at exit
across, say, `REPEAT=1` and `REPEAT=1000` means each `Terminal` cycle is
self-contained.

```sh
LIBGHOSTTY_FUZZ_REPEAT=1000 fuzz/afl/valgrind.sh
```

### Running the example

```sh
cargo run -p ghostling_rs
cargo run -p grid_ref_tracked_rs
```

When building with `link-dynamic`, set `LD_LIBRARY_PATH` on Linux or
`DYLD_LIBRARY_PATH` on macOS to the directory containing the generated
`libghostty-vt` shared library.
