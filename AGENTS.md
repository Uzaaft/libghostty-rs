# libghostty-rs

## Building

- Requires Zig 0.15.x on PATH and a Nix dev shell
- Enter dev shell: `nix develop`
- Check: `cargo check`
- Test: `cargo test -p libghostty-vt-sys`
- Build example: `cargo build -p ghostling_rs`
- Run example: `cargo run -p ghostling_rs`

## Code Conventions

- Rust workspace: `libghostty-vt-sys` (FFI bindings), `libghostty-vt` (safe wrappers), `ghostling_rs` (example)
- Opaque pointer pattern: `NonNull<ffi::GhosttyFoo>` + `PhantomData<*mut ()>` + `Drop`
- Sized structs: set `size` field to `std::mem::size_of::<Type>()` before FFI calls
- `from_result()` maps `GhosttyResult` to `Result<(), Error>`
- Ghostty source is fetched at build time by `build.rs` (pinned commit). Override with `GHOSTTY_SOURCE_DIR` env var to use a local checkout.
- Comment heavily — explain *why*, not just *what*
