# Rust WebAssembly terminal example

A port of Ghostty's [wasm-vt example](https://github.com/ghostty-org/ghostty/tree/521cf2c5d30bd48028ca61546c64e7913764073d/example/wasm-vt).
It keeps the upstream page layout, input and terminal flow: create a terminal,
write VT data, then show the contents as plain text. This is not an interactive
terminal emulator or a styled terminal renderer.

The terminal and formatter now live in Rust. JavaScript copies input into Wasm
memory and copies the output back before Rust frees it. It no longer needs to
read Ghostty's type manifest or build C structs by hand. These host exports
belong only to this example, not to the library crates.

## Build and run

From the repository root:

```sh
nix develop -c cargo build --locked --manifest-path example/Cargo.toml -p wasm_rs --target wasm32-unknown-unknown --release
```

Serve the repository root with a static HTTP server, for example:

```sh
python3 -m http.server 8000 --bind 127.0.0.1
```

Open `http://localhost:8000/example/wasm_rs/`. Do not open `index.html` directly
with `file://`: browsers block loading the module that way. The page loads
the release wasm32 build from `example/target/`.

Change the size or VT input and press **Run**. Each run creates a fresh
terminal. Input parsing follows upstream exactly: replace `\xNN`, then `\r`,
`\n`, `\t`, and finally doubled backslashes. The page keeps upstream's
size-input hints and error display, without adding JavaScript validation.

## Checks

`tests/smoke.rs` keeps the ABI, callback and allocator checks separate from the
demo. It runs as a native integration test and can also be built as a Wasm
module with `--example smoke`. `run.mjs` runs that module and tests the demo's
JavaScript/Rust bridge, including upstream's default input, Unicode, wrapping,
empty input and repeated calls after large allocations.

See the root [WebAssembly instructions](../../README.md#webassembly) for the
Node commands. CI runs both modules on wasm32 and wasm64 in debug and release.
The browser page uses wasm32 for compatibility; the same bridge handles wasm64
in Memory64-capable runtimes without a separate implementation.
