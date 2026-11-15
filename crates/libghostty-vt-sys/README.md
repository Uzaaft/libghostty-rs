# libghostty-vt-sys

Raw FFI bindings for libghostty-vt.

- Uses an installed library by default, discovered automatically through
  pkg-config. Static builds probe `libghostty-vt-static`; shared builds probe
  `libghostty-vt`. Missing libraries stop the build without downloading anything.
- Set both `GHOSTTY_LIB_DIR` and `GHOSTTY_INCLUDE_DIR` to use an installation
  without pkg-config. The header directory must contain `ghostty/vt.h`.
  The library must match the Rust target and the checked-in bindings.
- Enable `vendored` to compile a static library from the pinned sources with
  Zig 0.17.x. This takes precedence over installed libraries and enables the
  HTTP, checksum, and archive dependencies. No prebuilt libraries are downloaded.
- Vendored builds download the official source tarball through `ureq` with
  Rustls, verify its pinned SHA-256 checksum, and cache it under Cargo's `OUT_DIR`.
  No build path requires Git.
- With `vendored` enabled, set `GHOSTTY_SOURCE_DIR` to use local Ghostty sources
  instead of downloading the tarball.
- Set `ZIG` to an executable path or command name, without arguments, to override
  the default `zig` executable resolved through PATH.
- Exposes checked-in generated bindings in `src/bindings.rs`.
- Static linking is the baseline rather than a Cargo feature. Enable
  `link-dynamic` to link an installed shared library instead. `vendored` and
  `link-dynamic` are mutually exclusive. Windows DLLs must be next to the
  executable or on PATH at runtime.
- Set `GHOSTTY_ZIG_SYSTEM_DIR` to force Zig package resolution through a
  pre-fetched `zig build --system` directory. This is intended for Nix and other
  sandboxed package managers that cannot fetch during build scripts.
- Vendored builds target Zig's portable `baseline` CPU by default. Set
  `LIBGHOSTTY_VT_SYS_CPU` to `native`, a named CPU model such as `x86_64_v3`, or
  another Zig CPU expression to optimize for known deployment hardware.
- Set `LIBGHOSTTY_VT_SYS_OPTIMIZE` to `Debug`, `ReleaseSafe`, `ReleaseFast`, or
  `ReleaseSmall` to override the Zig optimize mode used by vendored builds.
- Vendored iOS targets (`aarch64-apple-ios`, `aarch64-apple-ios-sim`) build through
  ghostty's xcframework emit instead of a flat cross build. This requires a
  macOS host with Xcode and the iOS SDK installed. The simulator library is
  arm64-only, so `x86_64-apple-ios` is not supported by vendored builds.
