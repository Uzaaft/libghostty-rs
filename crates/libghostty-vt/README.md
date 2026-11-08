# libghostty-vt

Idiomatic, safe Rust bindings for [`libghostty-vt`](https://lib.ghostty.org/vt),
a portable, high-performance C library for terminal emulation. It is
[open source](https://github.com/ghostty-org/ghostty) and
[MIT licensed](https://github.com/ghostty-org/ghostty/blob/main/LICENSE).

- **It is highly generic:** you give it the bytes a program writes, and it
  processes and exposes the state of the terminal such as the screen, the
  cursor, the scrollback, etc.
- **It is highly portable:** we build and test libghostty-vt for Linux, macOS,
  iOS, Android, FreeBSD, Windows, WebAssembly, and bare-metal RISC-V and ARM. It
  works on 32-bit and 64-bit systems.
- **It is high-performance:** the IO throughput is best-in-class and faster than
  any other known terminal emulator. Memory usage is highly optimized, one
  libghostty-vt consumer was able to run 10,000 active (input/output) terminals
  with a running shell on a 4 core 8 GB Linux machine.
- **It has no dependencies:** For the highest performance, the library only
  needs libc, but you can optionally build it without that requirement and it
  can be fully freestanding on its own and run directly on hardware.

You can use libghostty-vt to build:

- Terminal emulators and multiplexers
- Web terminals and remote sessions
- Session recording and playback
- Test harnesses for TUI apps
- Agents and automation that read what a TUI is showing
- Etc.

libghostty-vt was originally extracted from [Ghostty](https://ghostty.org), but
since then is in use by
[hundreds of projects](https://github.com/Uzaaft/awesome-libghostty) both free
and commercial. We don't have any tracking of course but by our best estimates
libghostty-vt is used by tens of millions of people daily across every major
desktop and mobile platform.

## Where To Next?

We are currently writing a full book mirroring libghostty-vt's existing
[documentation](https://lib.ghostty.org/vt/), but with Rust bindings, Rust
examples and best practices. In the mean time, feel free to check out
the full [API reference on docs.rs](https://docs.rs/libghostty-vt).

## Example

[`ghostling-rs`](https://github.com/Uzaaft/libghostty-rs/blob/master/example/ghostling_rs/src/main.rs)
is a minimal yet functional terminal emulator built with `libghostty-vt`,
written as a single file with only around 1000 lines of heavily commented,
easily digestible code. It is based on the original [Ghostling](https://github.com/ghostty-org/ghostling)
which is built on the same underlying C API.
Other examples are currently work-in-progress — if you have any ideas for
examples, feel free to share them with us as an issue or pull request.
