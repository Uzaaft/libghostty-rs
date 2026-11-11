# The Grid {#guide-grid}

The grid is what's on the terminal: rows of cells, each holding text and a
style. This page is how you read it.

The hard part isn't reading a cell. It's naming one. "Row 3" means one thing to
a program moving its cursor, another to someone scrolled back through history,
and another to a tool walking the whole scrollback. And the grid moves under
you. A write can scroll a row into history, a resize can reflow it, and pruning
can throw it away.

So there are two ideas here. A point names a position in one of four coordinate
systems. A grid reference is what a point resolves to: one actual cell. An
untracked reference is a snapshot that dies on the next change to the terminal.
A tracked reference follows its cell, and the terminal pays a little for that on
every change.

This isn't how you draw. Grid references aren't built to sustain the framerates
needed to render a large screen. Use [Render State](crate::render) for that.
If you only want the text out, use [Formatter](crate::fmt). This page is for
reading particular cells, and for anything that has to keep pointing at one.

## Coordinates {#guide-grid-coordinates}

A GhosttyPoint is a coordinate plus a tag that says which coordinate system it's
in:

| Tag                         | Top-left is                    | What it covers                                                      |
| --------------------------- | ------------------------------ | ------------------------------------------------------------------- |
| [`point::Space::Active`]   | The top of the active area     | The area where the cursor can move, at the terminal's full height.  |
| [`point::Space::Viewport`] | The top of what you're showing | The visible viewport. This changes when it's scrolled.              |
| [`point::Space::Screen`]   | The oldest row of scrollback   | The full screen including scrollback, through the last written row. |
| [`point::Space::History`]  | The oldest row of scrollback   | Scrollback only. It ends at the row just before the active area.    |

Here's how they line up on a 24-row terminal with 100 rows of scrollback:

```
                    screen y   history y   active y
+----------------+
| scrollback     |         0           0
|                |       ...         ...
|                |        99          99
+----------------+
| active area    |       100                      0
|                |       ...                    ...
|                |       123                     23
+----------------+
```

The viewport is a 24-row window onto all of that. Scrolled to the bottom, it's
the same rows as the active area. See [Scrollback](scrollback.md#guide-scrollback)
for moving it.

`x` is the column and `y` is the row, and both start at zero. `y` is 32 bits
wide because a row in scrollback can be a long way from the top.

[`Terminal::grid_ref()`] resolves a point to a grid reference:

```rs
use libghostty_vt::point::{Point, Coordinate};

let point = Point::Active(Coordinate { x: 4, y: 0 });
if let Ok(grid_ref) = terminal.grid_ref(point) {
    let cell = grid_ref.cell().unwrap();
}
```

A point that's out of bounds returns [`Error::InvalidValue`]. Lookups with the
active and viewport tags are fast. The screen and history tags may have to walk
the full scrollback page list to find the row, so they can be expensive with a
large scrollback.

[`Terminal::point_from_grid_ref()`] goes the other way. Not every cell exists
in every coordinate system, though. A cell in scrollback has no active
coordinates, and a cell that has scrolled out of view has no viewport
coordinates. In those cases you get `None`. This is the only way to
get a cell's position.

## Cells, rows and styles {#guide-grid-cells}

A grid reference gets you everything about its cell:

| Function                         | What you get                                                    |
| -------------------------------- | --------------------------------------------------------------- |
| [`GridRef::cell()`]          | The [`Cell`].       |
| [`GridRef::row()`]           | The [`Row`] it's on.  |
| [`GridRef::graphemes()`]     | The full grapheme cluster, as codepoints.                       |
| [`GridRef::hyperlink_uri()`] | The hyperlink URI, if it has one.                               |
| [`GridRef::style()`]         | The [`Style`].                                               |

A [`Style`] describes how a cell looks: its foreground, background and
underline colors, plus flags for bold, italic, underline and other decorations.
A cell only holds a style id, [`Cell::style_id()`]. Use [`GridRef::style()`]
to get the full style.

This walks the whole grid, reading each cell's text, each row's wrap state, and
a style:

```rs
{{#include ../../../example/rust-vt-grid-traverse/src/main.rs:grid-ref-traverse}}
```

Full program: [rust-vt-grid-traverse/src/main.rs](/example/rust-vt-grid-traverse/src/main.rs)

## Untracked references {#guide-grid-untracked}

A [`GridRef`] is untracked. It's a value that snapshots one cell, and you
don't free it.

It's only valid until the next call that changes the terminal. That includes
[`Terminal::vt_write()`] and [`Terminal::resize()`]. There's no guarantee it
survives any change, even one to a part of the grid that looks unrelated.
So get one, read what you need, and cache the values. Don't keep the reference.

An untracked reference costs you the initial lookup and nothing after that. It's
a one-time snapshot, so it has no effect on the terminal's ongoing performance.

## Tracked references {#guide-grid-tracked}

A [`TrackedGridRef`] follows its cell. Scrolling, scrollback pruning, resize
and reflow, and other terminal changes all update it automatically. Create one
with [`Terminal::track_grid_ref()`].

Each tracked reference adds bookkeeping to every terminal change, so use them
sparingly. They're for long-lived anchors such as selections, search state,
marks and bookmarks in your application.

To read cell data, snapshot it first with [`TrackedGridRef::snapshot()`].
What you get back is an untracked reference with the same short life as any
other. [`TrackedGridRef::point()`] gets you the position without that step.

```rs
{{#include ../../../example/rust-vt-grid-ref-tracked/src/main.rs:grid-ref-tracked}}
```

Full program: [rust-vt-grid-ref-tracked/src/main.rs](/example/rust-vt-grid-ref-tracked/src/main.rs)

### When a tracked reference loses its cell

A tracked reference can still lose its place. That happens when the grid under
it is reset, pruned, or otherwise discarded in a way that can't be mapped to a
meaningful new cell. [`TrackedGridRef::has_value()`] then returns false, and
[`TrackedGridRef::snapshot()`] and [`TrackedGridRef::point()`] return [`None`].

The handle is still valid. Move it to a new point with [`TrackedGridRef::set()`].

### Primary and alternate screens

A tracked reference belongs to the screen that was active when you created it or
last set it. Converting it to a point uses that screen, even if the terminal has
since switched between primary and alternate. [`TrackedGridRef::set()`]
resolves the new point against the screen that's active now, so it can move the
reference from one screen to the other.

### Dropping

Tracked references are yours, and you can drop them after the terminal is gone.
If the terminal is dropped first, the handle stays valid for the tracked reference
functions only: it reports no value, and it can still be freed.

## When a reference dies {#guide-grid-lifetime}

|                       | Untracked                      | Tracked                                                        |
| --------------------- | ------------------------------ | -------------------------------------------------------------- |
| Terminal changes      | Invalid. Don't touch it again. | Follows the cell.                                              |
| Its cell is discarded | Invalid.                       | Valid, but has no value until you set it again.                |
| Terminal is freed     | Invalid.                       | Valid, but has no value. You still have to free it.            |
| Cost                  | One lookup.                    | Bookkeeping on every terminal change for as long as it exists. |
