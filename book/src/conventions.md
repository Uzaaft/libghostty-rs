# API Conventions {#conventions}

The libghostty-vt API follows the same few rules everywhere. Learn them once and
every bit of functionality in the library behaves in a predictable, regular
way.

Four of the rules are short and they're on this page. The others have pages of
their own:

- [Getting and Setting Values](values.md)
- [Sized Structs](sized-structs.md)
- [Memory](memory.md)
- [Optional Features](features.md)
- [Stability](stability.md)

## Results {#conventions-results}

Functions that can fail return a [`libghostty_vt::error::Result`]. The error
can be of the following:

| Result                      | What it means                                                                                                       |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| [`Error::OutOfMemory`] | An allocation failed.                                                                                               |
| [`Error::InvalidValue`] | An argument is wrong, such as a NULL pointer where a value is required or an enum value that is out of range.       |
| [`Error::OutOfSpace`]   | The buffer you gave us is too small. See @ref conventions-buffers.                                                  |
| [`Error::IoError`] | Reading from or writing to external I/O failed.                                                                     |
| [`Error::LimitExceeded`] | Encoded input went past a configured limit.                                                                         |
| [`Error::Rejected`] | A safety check refused the operation, such as pasted text that could inject commands. Nothing was done.             |
| [`Error::Unsupported`] | This library can't do what you asked. The key is newer than the library, or the feature was left out of this build. |

[`Error::Unsupported`] is how a program handles a library that isn't the one it was
built for. See [Newer Headers, Older Libraries](values.md#values-older-libraries) and
[Optional Features](features.md).

## Handles {#conventions-handles}

Much of the libghostty-vt API is handle-based. A handle is an opaque pointer
to something the library manages; the structure of the thing is not public
API for you to dig into. For example: [`Terminal`].

A handle you create is yours. Most come from a `new` function, such as
[`Terminal::new`]. If you need to, you can also explicitly pass an allocator
when allocating memory for the object with the `new_with_alloc` functions,
such as [`Terminal::new_with_alloc`]. See [Memory](memory.md).

A borrowed handle belongs to something else, so you don't free it. It also
doesn't last. The [`kitty::Graphics`] handle you get from
[`Terminal::kitty_graphics`] is only valid until the next call that changes the
terminal, such as [`Terminal::vt_write`]. Each borrowed handle documents
how long it lasts.

Some handles you create still point at a terminal, and it matters which one you
drop first:

- A [`Formatter`] borrows its terminal. The terminal must outlive the formatter.
- A [`Search`](search::Search) or a [`TrackedGridRef`](screen::TrackedGridRef) can
  outlive its terminal. If the terminal is dropped first, a search fails cleanly on
  any call that needs the terminal, a tracked reference reports no value, and you can
  still drop both.

Memory works the same way as handles. [`Bytes`](alloc::Bytes) represents a buffer
the library allocates for you, such as the output of [`fmt::Formatter::format_alloc`]
which is owned by the caller. See [Allocating and Freeing Buffers](memory.md#memory-buffers).

## Threads {#conventions-threads}

The library creates no threads on its own. In general, calls on one handle
are never thread-safe. In the Rust bindings, this is represented by making
handle types `!Send + !Sync`. But calls across multiple handles are. It is
possible to build a multi-threaded terminal on libghostty-vt and is how many
terminals are built.

Every callback runs on the thread whose call triggered it. libghostty-vt
has no concept at all of main threads or routing calls to different threads;
you must solve this yourself as the caller.

Calls on one handle, and on any handle borrowed from it, must not run at the
same time unless a function's documentation says otherwise. Handles that share
nothing can be used on different threads.

A common example: render state can run on a separate thread from terminal IO,
so that IO can continue while rendering happens. There is only one function
in render state that uses the terminal and requires a lock.

## Output Buffers {#conventions-buffers}

Functions that write bytes for you take a mutable slice. You usually don't
know how much space the output needs, so you can ask. Pass an empty slice and
the call tells you the size.

When there's nothing to write, the call returns `Ok(0)`, even for a size query.

When the buffer is too small, the call returns an [`Error::OutOfSpace`], which
contains the size needed. The contents of the buffer are then unspecified unless
the function says otherwise. Throw them away and retry with a larger buffer.
