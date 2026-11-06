# About libghostty-vt

> [!IMPORTANT]
>
> This book is an adaptation of the original C language
> [libghostty-vt documentation](https://lib.ghostty.org/vt), using the
> officially supported [Rust bindings](https://crates.io/crates/libghostty-vt)
> and slightly altered to reflect how it should idiomatically be used in Rust.
>
> While the two sources should be largely consistent, the C documentation should
> be seen as the more authoritative source in case of conflicting information.

libghostty-vt is a portable, high-performance C library for terminal emulation,
with officially supported bindings for [Rust](https://crates.io/crates/libghostty-vt)
and more. It is [open source](https://github.com/ghostty-org/ghostty) and
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

> [!IMPORTANT]
> libghostty-vt is pre-1.0. The core code is extremely stable and
> already in use by millions worldwide, but we're still stabilizing the ABI.
> We will ship a stable ABI with 1.0 soon. See [Stability](stability.md) for exactly what
> can break.

## Where to Next?

- Getting the library? See [Installing](getting-started/installing.md).
- Building a terminal? Read [Your First Terminal](getting-started/first-terminal.md).
- Only need one piece? The parsers and encoders work without a terminal. See
  @ref api_building_blocks "Building Blocks" and @ref api_input "Input".
- Read @ref conventions for general library-wide conventions.

## Example

Here is a complete program. It creates a terminal, writes some text with a color
sequence in it, and prints what ended up on the screen. We left out the error
checks to keep it short.

@snippet c-vt-hello/src/main.c hello

Full program: @ref c-vt-hello/src/main.c

It prints `Hello, world!` in plain text. The color is in the terminal's state
but the way we configured the formatter doesn't show it. This has no GUI window
and doesn't use a real PTY, but it is hopefully demonstrative of libghostty-vt
and how you could hook it up to both.
