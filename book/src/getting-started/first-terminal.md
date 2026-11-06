# Your First Terminal {#first-terminal}

Let's create a simple terminal that processes real bytes and produces a rendered
screen. There's no GUI or real pty attached to it, to keep the example simple
and focused on libghostty-vt.

Every feature in libghostty-vt builds on the basics shown here: the function
call style, error handling, and option setting. If you go through this page,
then you'll be able to easily adopt the remainder of the library.

We'll walk through @ref rust-vt-stream/src/main.rs:"c-vt-stream". [Installing](installing.md)
shows how to build it, and in the repo it's one `cargo run`.

## Create a Terminal {#first-terminal-create}

```rs
{{#include ../code/rust-vt-stream/src/main.rs:vt-stream-init}}
```

[`Terminal::new`] takes the size in columns and rows. If you want to use a
custom allocator, use [`Terminal::new_with_alloc`]. See [Memory](../memory.md).

libghostty-vt functions never crash your program by design, given valid use of
the API. All failure scenarios including allocation failures are handled and
returned, and libghostty-vt never puts itself into a corrupt state. Always check
the [`Result`](error::Result) of every function.

## Write Bytes {#first-terminal-write}

```rs
{{#include ../code/rust-vt-stream/src/main.rs:vt-stream-write}}
```

[`Terminal::vt_write`] takes the raw bytes a program wrote (e.g. from a
pty, session recording, network, etc.). libghostty-vt parses all escape
sequences and applies their logic appropriately to the terminal state. Above,
`\x1b[1;32m` turns on bold green. `\x1b[5B` moves the cursor down five rows.

This call never fails, which is why there's no result to check. You also don't
need to line your writes up with anything. A sequence or a UTF-8 character can
be split across two calls and the parser picks up where it left off.

It is safe to pass untrusted data to this function. Bad input gets skipped. By
default, without any [effects](guides/effects.md), untrusted data can't cause
any damage. Memory usage is bounded and no side effects are possible.
libghostty-vt is fuzzed 24/7 with multiple dedicated machines to keep this
function safe against all forms of data input.

Your program can widen this. Be careful with effects, such as clipboard reads,
and with the options that let Kitty images load from files or shared memory.
Anything you read back, such as the title, also came from untrusted data, so
treat it that way.

Note the end of each line is `\r\n`, not just `\n`. In terminal emulators, a
`\n` alone just moves the cursor down. It doesn't move it back to the first
column.

You rarely see this because a pty normally turns a program's `\n` into `\r\n` in
the kernel line discipline logic. But if you feed in bytes that didn't come
through a pty, such as a text file, that's now your job. Beware!

## Read the Screen {#first-terminal-read}

> [!WARNING]
> The formatter is only for simple snapshots of the screen, such as saving it
> to a file or printing it once. Don't use it for a real, live render.
> Use a render state instead. See [Draw the Screen](#first-terminal-render).

```rs
{{#include ../code/rust-vt-stream/src/main.rs:vt-stream-read}}
```

A terminal screen is a grid of cells with styles, wide characters, wrapped rows,
etc. The formatter processes this grid state and produces the screen state into
a format requested: plain text, HTML, VT sequences.

A new formatter by default emits plain text with trailing whitespace trimmed, so
the two `Formatter::set_*` calls here only show you where those options
are.

The program prints this:

```
Top-left corner
Green Text




Moved down!
New content
Line A
Line B
Line C
```

Notice what's missing. We wrote `Hello, World!` first, but `Top-left corner`
went on top of it after the cursor moved to row 1. This shows that the
formatter is processing a real terminal screen.

## Draw the Screen {#first-terminal-render}

The RenderState API exists to build a high-performance, low-latency,
frame-by-frame renderer. It can be used as the foundation for CPU or GPU
rendering. It does dirty tracking, viewport handling, smooth scroll handling,
and more.

```rs
{{#include ../code/rust-vt-stream/src/main.rs:vt-stream-render}}
```

A real renderer repeats three steps for every frame: update, draw, clean. Create
the handles once, outside of that loop.

[`RenderState::update`] is the only step that touches the terminal, which
creates a [`Snapshot`] of the render state. After that, you draw from the
snapshot alone, so your renderer doesn't get in the way of the program writing
to the terminal. The update only redoes what changed. Because only update needs
access to the terminal, this is the only critical area that requires a lock for
multi-threaded terminal and renderers.

> [!NOTE]
>
> Unlike the C API, in Rust we make a clear distinction between a [`RowIterator`]
> and a [`RowIteration`]. While the iterator can be reused over and over again,
> the *iteration* only reflects its current state, after being fed up-to-date data.
> This is to prevent users from accessing stale or invalid state when iterators
> aren't yet ready.
>
> This design is symmetrical across all three objects of the render state API:
>
> | Handle created on startup | State on each frame |
> |-------------------|-------------------|
> | [`RenderState`]   | [`Snapshot`]      |
> | [`RowIterator`]   | [`RowIteration`]  |
> | [`CellIterator`]  | [`CellIteration`] |

[`RowIteration::next_dirty`] skips the rows that didn't change, which is most of
them in most frames. Once you've drawn a complete frame, [`Snapshot::clean`]
marks it as done. An update never clears the dirty state, so without this call
every row stays dirty.

This program draws by printing, so you see the same screen as the formatter
printed, except that `Green Text` is now bold. It's still not green, because we
didn't read the colors. See [Rendering](../guides/rendering.md) for the colors, the cursor, and
how to share a terminal with a renderer thread.

## Drop Everything {#first-terminal-drop}

libghostty-vt does allocate at times, and they need to be dropped. Fortunately,
the Rust bindings use Rust's lifetime and RAII rules as much as possible to
drop them in the correct order.

The buffer from [`Formatter::format_alloc`] is yours. You can drop it whenever
you want, as long as the buffer does not outlive the allocator it that gave
its memory. See [Allocating and Dropping Buffers](../memory.md#memory-buffers).

The formatter borrows the terminal, so the terminal has to outlive it.
Drop the formatter first.

The render state and its iterators are yours too. The borrow checker should be
able to ensure they are dropped in order.

The terminal should be dropped last.

The rules for handles are the same across the whole library.
See [Handles](../conventions.md#conventions-handles).

## What to Read Next {#first-terminal-next}

- [Effects](guides/effects.md) - Answer the things a program asks of the terminal.
- @ref example-programs - Complete programs for everything else.
