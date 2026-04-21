# libghostty-vt-sys

Raw FFI bindings for libghostty-vt.

- Fetches and builds `libghostty-vt.so`/`.dylib` from ghostty sources via Zig.
- Exposes checked-in generated bindings in `src/bindings.rs`.
- Set `GHOSTTY_SOURCE_DIR` to force the build to use a local Ghostty checkout.
- If the `pkg-config` feature is enabled, the build will use an installed
  `libghostty-vt` found through `pkg-config` only when `GHOSTTY_SOURCE_DIR` is
  unset.
