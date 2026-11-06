# Snapshots {#guide-snapshots}

A snapshot is the complete state of a terminal as bytes. Encode one, keep it or
send it somewhere, and restore a terminal from it later.

The problem with a big terminal is scrollback. A complete snapshot can be large,
and you don't want someone staring at nothing while old history loads. So a
snapshot is ordered: first comes enough state to render and resume the terminal,
including any unfinished VT parser input, and then the older scrollback. You can
show a restored terminal and feed it live input while the history is still
arriving.

Most programs never need this. If you do, you can restore a snapshot in one
call, or in pieces if you want the terminal on screen first.

The format isn't stable yet. Snapshot format version 1 is a work in progress and
doesn't carry a binary-compatibility guarantee.

## Encoding {#guide-snapshots-encoding}

There are three ways to encode, and they differ only in where the bytes go:

| Function                        | Where the bytes go                                                     |
| ------------------------------- | ---------------------------------------------------------------------- |
| [`Terminal::encode_snapshot()`]       | A [writer](std::io::Write).                                                       |
| [`Terminal::encode_snapshot_buf()`]   | A buffer you provide. Pass an empty slice to ask for the size.        |
| [`Terminal::encode_snapshot_alloc()`] | A new, allocated buffer. |

A snapshot includes unfinished input, such as half of an escape sequence. To
capture it, the terminal has to have been keeping track of that input, which you
turn on by calling [`Terminal::set_continuation_max_bytes()`]. Turn it on before the
input is written. If tracking is off and the VT parser and UTF-8 decoder are
both at ground, encoding still works. If either one is unfinished, encoding
returns [`Error::InvalidValue`].

@snippet c-vt-snapshot/src/main.c snapshot-encode

Full program: @ref c-vt-snapshot/src/main.c

[`Terminal::encode_snapshot()`] needs the terminal to hold still. Don't write to it or
change it for the duration of the call, and don't call terminal functions on the
same terminal from your writer. Encoding begins at the writer's current
position, and calls to the writer are synchronous. We don't flush, or make your
destination durable. If an error occurs, the writer may hold a partial snapshot
with no valid FINISH marker.

## Restoring in one call {#guide-snapshots-restoring}

Create a decoder over your bytes with [`snapshot::Decoder::new_buf()`], then
call [`snapshot::Decoder::decode()`]. You get back a terminal that you own,
with its VT stream restored. It's safe to free the decoder right away.

The bytes aren't copied. They have to stay valid and unchanged until FINISH is
reached or the decoder is freed. Bytes after FINISH aren't consumed, and
[`snapshot::Decoder::source_offset()`] tells you where they start.

@snippet c-vt-snapshot/src/main.c snapshot-decode

Full program: @ref c-vt-snapshot/src/main.c

### Options

Set options with the various setters before decoding starts. After that it
returns [`Error::InvalidValue`]. A `None` value restores an option's default.

| Option                                                  | What it does                                                                                       |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| [`snapshot::Decoder::set_max_continuation_bytes()`]    | The largest unfinished input the decoder accepts. Zero accepts only snapshots at ground.           |
| [`snapshot::Decoder::set_retain_continuation()`]       | Keep tracking unfinished input on the restored terminal. Tracking is off by default.               |
| [`snapshot::Decoder::set_compress_history_enabled()`]          | Compress each history page right after restoring it.                                               |
| [`snapshot::Decoder::set_kitty_image_storage_limit()`] | The image storage limit for each restored screen. The snapshot doesn't carry the original's limit. |

If you retain the continuation and don't need tracking after you've exported it,
call [`Terminal::set_continuation_max_bytes()`] with zero before you write any
new input. Exporting an empty continuation doesn't turn tracking off.

Without [`snapshot::Decoder::set_compress_history()`], restored scrollback is
uncompressed, even if the original terminal had compressed it. That can be many
times more memory than the terminal needed before, and it stays that way until
you call [`Terminal::compress()`]. With the option on, the decode never holds
more than one uncompressed history page at a time. The snapshot format doesn't
change, so this works with any snapshot.

@snippet c-vt-snapshot/src/main.c snapshot-compress-history

Full program: @ref c-vt-snapshot/src/main.c

### Errors

End-of-file before a required READY or FINISH marker means the snapshot is
truncated, and you get [`Error::InvalidValue`]. [`Error::IoError`] is reserved for
a reader callback that returns false.

A decoding, I/O or allocation error after the decoder has started consuming
input poisons it, and all you can do is drop it. An invalid argument or
lifecycle error caught before any input is consumed doesn't.

## Restoring in pieces {#guide-snapshots-pieces}

This is the path that gets a terminal on screen first:

1. [`snapshot::Decoder::ready()`] decodes through READY and gives
   you an [`snapshot::IncrementalDecoder`].
   [`snapshot::IncrementalDecoder::terminal()`] gives you
   a reference to the underlying terminal, usable right away for rendering and live input.
2. [`snapshot::Decoder::next()`] decodes one record per call: a history page with the
   Kitty graphics placements pinned to it, or the pixels of one image or animation frame.
   The current progress is returned as a [`snapshot::Progress`] struct.
3. `Ok(None)` from [`snapshot:Decoder::next()`] means FINISH was
   validated. Calling it again returns the same thing.

History pages arrive newest to oldest, so each one is prepended to the
scrollback.

The source can be a [reader](std::io::Read) instead of a buffer. Create that decoder
with [`Decoder::new()`](snapshot::Decoder::new). It stores a reference to your reader,
and the callback and its context have to stay valid until FINISH is reached or the
decoder is freed. Reads are synchronous and only happen during ready, next and
decode calls.

A successful read of zero bytes is permanent end-of-file, not temporary
starvation. If your source is nonblocking, wait outside the decoder or block in
your reader implementation.

@snippet c-vt-snapshot/src/main.c snapshot-buffer-reader

Full program: @ref c-vt-snapshot/src/main.c

@snippet c-vt-snapshot/src/main.c snapshot-incremental

Full program: @ref c-vt-snapshot/src/main.c

### Between calls

You can render, resize and feed live pty input to the terminal between calls.
The decoder borrows your terminal while it restores history, so keep the
terminal alive until FINISH validates or the decoder is freed.

If a history page can't be applied safely anymore, it's still consumed and
validated, and progress reports zero rows. Image pixels work the same way, and
report zero bytes. That happens if live input deleted the image or sent a new
image with the same id, if the image didn't fit the storage limit, or if its
screen was removed or replaced.

[`snapshot::IncrementalDecoder::progress_remaining`] counts the page records left
in the same screen's history, not in the whole snapshot.

You can stop early. Freeing the decoder leaves you a usable terminal with
whatever history was already restored. After a decoding error the terminal is
also still yours and usable, but the only thing you can do with the decoder is
free it.

### Images

Images are restored at READY with their pixels pending. A pending image can be placed
and queried, but [`Image::data()`](kitty::graphics::Image::data) returns `Ok(None)` for it
until its pixels arrive through [`IncrementalDecoder::next()`](snapshot::IncrementalDecoder::next).

When they arrive, the image keeps its [`Image::generation()`](kitty::graphics::Image::generation), but [`Graphics::generation()`](kitty::graphics::Graphics::generation)
changes. A renderer that watches that value picks up the new pixels on its next frame.

Pixels can arrive between history pages, so they're only known to be done when
next returns `None`. To show progress, compare
[`snapshot::IncrementalDecoder::kitty_pending_bytes_primary`] with what arrives.
[`snapshot::IncrementalDecoder::next`] has an example.

## The format {#guide-snapshots-format}

You only need this if you're writing your own tools for snapshots. The
[Zig codec documentation](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/snapshot/main.zig)
has the rest.

A snapshot is an ordered, CRC-protected record stream. Every integer is unsigned
and little-endian. The stream begins with this fixed ten-byte envelope:

```
byte  0               8       10
      +---------------+--------+
      | "GHOSTSNP"    | version|
      | 8-byte magic  | u16    |
      +---------------+--------+
```

After that come independently checksummed records. A record's CRC32C covers its
encoded tag and payload length, followed by its payload. It doesn't cover the
CRC field itself.

```
byte  0       2             6          10             10 + payload_len
      +-------+-------------+-----------+----------------+
      | tag   | payload_len | CRC32C    | payload        |
      | u16   | u32         | u32       | payload_len B  |
      +-------+-------------+-----------+----------------+
      \____________________/             \______________/
         CRC prefix                         CRC suffix
```

Record groups occur in this strict order. SCREEN and HISTORY groups contain one
entry for each screen declared by TERMINAL. Each manifest is followed by the
number of PAGE records it declares. Active SCREEN pages make the terminal
renderable. HISTORY pages are older scrollback, ordered newest to oldest so an
incremental decoder can prepend them as they arrive.

```
+---------------- TERMINAL ----------------+
| terminal-wide state and screen count     |
+----------------- SCREEN -----------------+  repeated per screen
| active-screen manifest                   |
+------------------ PAGE ------------------+  repeated per manifest
| active screen rows                       |
+---------- KITTY_IMAGE_STORAGE -----------+  optional, per screen
| images, placements, in-progress load     |
+------------- CONTINUATION ---------------+
| unfinished VT/UTF-8 input, or ground     |
+------------------ READY -----------------+
| empty renderable-state marker            |  ready() returns here
+---------- KITTY_IMAGE_PAYLOAD -----------+  optional, anywhere until FINISH,
| one image's or frame's pixels            |  next() consumes one
+----------------- HISTORY ----------------+  repeated per screen
| scrollback manifest                      |
+------------------ PAGE ------------------+  next() consumes one page
| older screen rows                        |
+------ KITTY_IMAGE_PAGE_PLACEMENTS -------+  optional, after a PAGE
| placements pinned to that page           |
+------------------ FINISH ----------------+
| empty end-of-snapshot marker             |  next() returns NO_VALUE
+------------------------------------------+
| trailing transport bytes (not consumed)  |
+------------------------------------------+
```

READY separates the renderable prefix through CONTINUATION from history. FINISH
terminates the record sequence. Both are empty records protected by CRC32C, like
every other record. Declared record counts, tags and strict decoding enforce the
stream's ordering and completeness.

A decoder skips any record whose tag it doesn't know, wherever it appears. That
lets a newer library add optional kinds of records that older libraries ignore,
without changing the snapshot version. Skipped records are still checked, so a
damaged record fails decoding even when its tag is unknown.

The Kitty graphics records are optional in this way. A library built without
Kitty graphics skips them and restores the same terminal without its images, and
a terminal without images writes none of them.
