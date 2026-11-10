# Installing {#installing}

The Rust bindings for libghostty-vt are available on
[crates.io](https://crates.io/crates/libghostty_vt) as `libghostty_vt`.
However, since it's a C library implemented in Zig, installing it is not
as easy as just adding the crate to your project.

## pkg-config

For general use cases we recommend using libghostty-vt as a system library,
usually built by your distribution or package manager. The crate discovers
the C library with `pkg-config`, so any package manager should work.

The Rust crate defaults to static linking, but dynamic linking is available
via the `link-dynamic` feature flag. Other than that, the opportunity for
configuring libghostty-vt as a system library is rather limited, as it is
usually compiled with general-purpose settings. For more fine-grained control,
you might want to [vendor](#vendoring) libghostty-vt instead.

## Vendoring

When you have more specific requirements, such as finetuning the list of included
[Optional Features](../features.md) in order to minimize binary size, using
libghostty-vt in particular hardware environments (embedded SoCs, retro hardware,
etc.), and so on, you might want to build libghostty-vt from source as a vendored
library. Opting into vendoring is done with the `vendored` Cargo feature.

You then need to have [Zig](https://ziglang.org) installed and available
in your `PATH` (see [Building from Source] in the C documentation). The
crate's [build script](/crates/libghostty-vt-sys/build.rs) would then download
libghostty-vt's source tarball, build with the appropriate flags, and then
link your project to the built library. Zig features great support for
cross-compilation out of the box, so feel free to compile against any
architecture you like.

Static linking is the default, but dynamic linking is supported on most
platforms. One notable exception is on iOS where everything has to be
statically linked, and the build host must be macOS with both Xcode and
iOS SDKs installed. This is a restriction from Apple that we cannot easily lift.

[Building from Source]: https://lib.ghostty.org/vt/installing.html#installing-source
