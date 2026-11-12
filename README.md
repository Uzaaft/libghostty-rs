# libghostty-rs

Rust bindings and safe API for [libghostty-vt](https://ghostty.org), the virtual terminal emulator library extracted from [Ghostty](https://ghostty.org).

## Workspace Layout

- `crates/libghostty-vt-sys` — raw FFI bindings generated from `ghostty/vt.h`
- `crates/libghostty-vt` — safe Rust wrappers (Terminal, RenderState, KeyEncoder, MouseEncoder, etc.)
- `example/grid_ref_tracked_rs` — focused example of tracked grid references following cells through scrollback and reset
- `example/ghostling_rs` — Rust port of [ghostling](https://github.com/ghostty-org/ghostling), a minimal terminal emulator using [macroquad](https://macroquad.rs)

## Quick Start

```rust
use libghostty_vt::{Terminal, RenderState};
use libghostty_vt::render::{RowIterator, CellIterator};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a terminal with 80 columns and 24 rows.
    let mut terminal = Terminal::new(80, 24)?;

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

By default, both crates use an installed libghostty-vt. The build automatically
probes `libghostty-vt-static` through `pkg-config`. It does not download anything
or require Zig or Git. If discovery fails, install the library or choose one
of the alternatives below. The checked-in bindings must match the installed
C API; arbitrary library revisions are not guaranteed to work.

For an installation without pkg-config, set both `GHOSTTY_LIB_DIR` and
`GHOSTTY_INCLUDE_DIR` to the library and header directories. These override
pkg-config discovery and support split installations such as Nix outputs.
The header directory must contain `ghostty/vt.h`. When cross-compiling, supply
a library for the target, not the host.

Enable `vendored` to compile the pinned sources into a static library with Zig:

```toml
libghostty-vt = { version = "0.2.1", features = ["vendored"] }
```

This feature takes precedence over installed libraries. It requires
[Zig](https://ziglang.org/) 0.17.x on PATH, or selected through `ZIG`.
Set `ZIG` to an executable path or command name, without arguments, to override
the default `zig` executable. The build downloads the official source tarball
through `ureq` with Rustls and verifies its pinned SHA-256 checksum. Verified
archives and extracted sources are cached under Cargo's `OUT_DIR`. Network or
checksum failures stop the build. No build path requires Git, and no prebuilt
libraries are downloaded.

With `vendored` enabled, set `GHOSTTY_SOURCE_DIR` to use local Ghostty sources
instead of downloading them. Package managers that need network-free builds
can also set `GHOSTTY_ZIG_SYSTEM_DIR` to a pre-fetched Zig package directory.
This is passed to `zig build --system` so Zig does not download dependencies.
The HTTP, checksum, and archive dependencies are enabled only by `vendored`.

Vendored builds derive Zig's optimize mode from Cargo's profile: dev builds use
`Debug`, size-optimized builds use `ReleaseSmall`, and other release builds use
`ReleaseFast`. Set `LIBGHOSTTY_VT_SYS_OPTIMIZE` to `Debug`, `ReleaseSafe`,
`ReleaseFast`, or `ReleaseSmall` to override that choice explicitly.

Vendored builds target Zig's portable `baseline` CPU so published binaries can
run on older processors. For a binary that will run on the build machine, set
`LIBGHOSTTY_VT_SYS_CPU=native`; a named Zig CPU model such as `x86_64_v3` can be
used when all deployment machines support that target.

Native Nix builds and the default dev shell provide upstream's pinned
`libghostty-vt` package through `buildInputs`. Cargo discovers it automatically
through pkg-config, with no extra Cargo feature. Nix can substitute the library
from Ghostty's binary cache when available. Wasm and the explicit iOS vendored
check build from source instead.

Downstream Nix source builds can enable `vendored`, prefetch the pinned Ghostty
source and its Zig dependencies, then set `GHOSTTY_SOURCE_DIR` and
`GHOSTTY_ZIG_SYSTEM_DIR` rather than allowing network access in the sandbox.

By default, `libghostty-vt` and `libghostty-vt-sys` link `libghostty-vt.a`.
This statically links the Ghostty VT archive, but the final binary may still
depend on platform runtime libraries. To link the shared library instead,
enable `libghostty-vt/link-dynamic`. It probes the `libghostty-vt` pkg-config
module instead. `vendored` and `link-dynamic` are mutually exclusive.
On Windows, put the shared library DLL next to the executable or on PATH.
An explicitly supplied static archive must match the target's linker; the
upstream GNU release archive does not support MSVC's `link.exe`.

```sh
nix develop
cargo check
cargo test -p libghostty-vt-sys
cargo build --manifest-path example/Cargo.toml --workspace
```

### Regenerating bindings

`bindings.rs` and `layouts.rs` in `crates/libghostty-vt-sys/src/` are checked
in and must match the Ghostty commit pinned in `crates/libghostty-vt-sys/build.rs` and
`flake.nix` (run `nix flake update ghostty` after changing the latter); the
`bindings-fresh` Nix check fails otherwise. After bumping the pin, regenerate
the bindings from the dev shell:

```sh
cargo run -p libghostty-vt-sys --features bindgen-tool --bin gen-bindings && cargo fmt -p libghostty-vt-sys
```

The generator reads the headers from `GHOSTTY_INCLUDE_DIR`, or from
`GHOSTTY_SOURCE_DIR/include`, and otherwise from the headers the build script
installed for that same build. It parses them for each vendored target using
explicit Clang targets and verifies that the API declarations are identical.
`layouts.rs` holds compile-time size, alignment and field-offset assertions,
gated by the target ABI reported by `rustc`. Targets share checks only when
Clang generated identical assertions. Adding a vendored target also requires
adding it to the generator's `TARGETS` list; unlisted targets receive no layout
checks. Regeneration needs no target SDKs or installed cross-target Rust standard
libraries, and ordinary builds need neither Clang nor the generator.

### Miri Verification

Run the Rust-owned soundness checks with:

```sh
nix run .#miri
```

The command supplies its own nightly Miri toolchain and checks both binding and
wrapper crates. These checks intentionally stay on Rust-owned unsafe seams such
as string, slice, and allocator plumbing. They do not execute the native
Ghostty FFI backend.

Use `nix develop .#miri` when working interactively with the same toolchain.

### WebAssembly

Wasm support is selected by the Rust target, not a Cargo feature. The existing
Rust API links Ghostty's static archive into the application's Wasm module;
no JavaScript bindings or browser dependencies are added to either library crate.
Do not enable `link-dynamic` on Wasm.

| Rust target | Zig target | Toolchain |
| --- | --- | --- |
| `wasm32-unknown-unknown` | `wasm32-freestanding` | Stable Rust, including the MSRV |
| `wasm64-unknown-unknown` | `wasm64-freestanding` | Nightly with `rust-src` and `-Zbuild-std=std,panic_abort`; a Memory64-capable runtime |

The pinned Ghostty revision builds both freestanding architectures. Its
`wasm32-wasi` build fails because its panic/stderr path requires libc without
linking it, and its standalone Emscripten module fails to link libc. WASI
(including Rust's preview variants) and Emscripten are not supported by this
integration until those upstream build paths work. `wasm32v1-none` is a
`no_std` Rust target; these crates require `std`. Wasm does not imply support
for Rust threads, filesystem access, or networking.

The [browser example](example/wasm_rs/README.md) ports upstream's `wasm-vt`
demo to the Rust API. Its ABI and allocator smoke tests are separate from
the demo. Run both with Node.js (provided by the dev shells):

```sh
nix develop -c cargo build --locked --manifest-path example/Cargo.toml -p wasm_rs --lib --example smoke --target wasm32-unknown-unknown --features wasm_rs/vendored --release
nix develop -c node example/wasm_rs/run.mjs example/target/wasm32-unknown-unknown/release/examples/smoke.wasm example/target/wasm32-unknown-unknown/release/wasm_rs.wasm

nix develop .#wasm64 -c cargo build -Zbuild-std=std,panic_abort --locked --manifest-path example/Cargo.toml -p wasm_rs --lib --example smoke --target wasm64-unknown-unknown --features wasm_rs/vendored --release
nix develop .#wasm64 -c node example/wasm_rs/run.mjs example/target/wasm64-unknown-unknown/release/examples/smoke.wasm example/target/wasm64-unknown-unknown/release/wasm_rs.wasm
```

CI also executes Debug builds. Ghostty's Debug Wasm build imports
`env.log(pointer, length)`; the example runner supplies it. Its release build
discards logs instead of calling the native logger hook. Embedders must
register a cryptographically secure source through
`sys::set_secure_random_source` before using operations that require entropy;
freestanding Ghostty has no platform entropy source. Browser applications can
supply Web Crypto from their own host integration.

The raw `ghostty_wasm_*` allocation helpers are available through
`libghostty-vt-sys` on Wasm. Rust callers normally use the existing owned types
and allocator APIs instead; the helpers are intended for hosts managing raw
Wasm memory. Do not mix their allocations with `ghostty_free` or Rust's heap.

### Running the example

The examples live in their own workspace under `example/`. To run one, enter
its folder and run:

```sh
cargo run
```

With `link-dynamic`, `cargo run` finds the shared library by itself. To run a
built example directly instead, set `LD_LIBRARY_PATH` on Linux or
`DYLD_LIBRARY_PATH` on macOS to the directory containing the installed
`libghostty-vt` shared library.
