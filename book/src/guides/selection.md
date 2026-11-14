# Selection {#guide-selection}

A selection is a range of terminal content: a word someone double-clicked, a
line, a command's output, or a block dragged out with the mouse. You make one to
highlight text and to copy it.

Selecting text in a terminal takes more than two cells. What counts as a word,
where a line really starts, which rows are a command's output, and what a
double-click-and-drag should do when the pointer is between words are all
questions we've already answered in Ghostty. These functions give you the same
answers.

If you don't let anyone select text, you can skip this page. If you do, the
gesture section is the one you want. It turns raw pointer events into selections
so you don't have to write that logic.

## What a selection is {#guide-selection-what}

A [`Selection`] is two cells, `start` and `end`, and both are included. By
default it's linear: it runs from `start` to `end` the way text flows, wrapping
from the end of one row to the start of the next. Set `rectangle` to true for a
rectangular block instead, with `start` and `end` at opposite corners.

The ends keep the direction of the selection. If someone drags up or to the
left, `start` comes after `end` in the terminal, so don't assume `start` is the
top-left end. [`Selection::to_ordered()`] gives you the ends in the
order you ask for.

Both ends are [`GridRef`] values, so a selection is an untracked snapshot and
follows the same rule as any untracked reference. It's only valid until the next
call that changes the terminal, including freeing it.
See [When a reference dies](grid.md#guide-grid-lifetime).

To keep a selection across changes, you have two choices. Install it as the
terminal's selection, or keep tracked references for the ends and build a new
[`Selection`] from fresh snapshots when you need one.

### The terminal's selection

The active screen has a selection, and the terminal owns it. Passing a selection
to [`Terminal::set_selection()`] copies it right away and converts it to tracked
state, so what you passed doesn't need to outlive the call. Passing `None` clears
it.

Read it back with [`Terminal::selection()`], which returns `None` if there's no
active selection. The struct you get is yours to keep, but the references in it
are untracked again. They're only valid until the next call that changes the
terminal.

Nothing in this guide installs a selection for you. Every function below returns
a snapshot, and you decide what to do with it.

## Making a selection {#guide-selection-making}

You can fill in a selection by hand with [`Terminal::select`] (or equivalently,
[`Selection::new`]). This selects the first ten cells of the second row and
makes that the terminal's selection:

```rs
use libghostty_vt::{
    point::{Coordinate, Point},
    selection::Selection,
    Terminal,
};

let start = terminal.grid_ref(Point::Active(Coordinate { x: 0, y: 1 }))?;
let end = terminal.grid_ref(Point::Active(Coordinate { x: 9, y: 1 }))?;
let selection = terminal.select(start, end, false);
terminal.set_selection(Some(selection))?;
```

Usually you want the terminal's rules instead:

| Function                               | What it selects                                                                    |
| -------------------------------------- | ---------------------------------------------------------------------------------- |
| [`Terminal::select_word()`]         | The word under a grid reference.                                                   |
| [`Terminal::select_word_between()`] | The nearest word between two grid references, searching from `start` toward `end`. |
| [`Terminal::select_line()`]         | The line under a grid reference.                                                   |
| [`Terminal::select_output()`]       | The command output under a grid reference. This needs OSC 133 prompt markers.      |
| [`Terminal::select_all()`]          | All selectable content.                                                            |

Each returns `None` when there's nothing to select there.

The word functions use Ghostty's default word boundaries unless you pass your
own codepoints. The line function trims Ghostty's default whitespace unless you
pass your own. Set `semantic_prompt_boundary` and the line stops at semantic
prompt changes, so a line at a shell prompt selects the input and leaves out the
`$ `.

```rs
{{#include ../../../example/rust-vt-selection/src/main.rs:selection-main}}
```

Full program: [rust-vt-selection/src/main.rs](/example/rust-vt-selection/src/main.rs)

### Double-click and drag

Word-granular dragging needs [`Terminal::select_word_between()`]. If someone
double-clicks a word and drags across spaces or punctuation toward another word,
selecting only the word under the pointer flickers or collapses whenever the
pointer is between words. So ask for the nearest word between the click and the
drag point, ask again in the other direction, and combine the two. The gesture
API below does this for you.

```rs
{{#include ../../../example/rust-vt-selection/src/main.rs:selection-word-between}}
```

Full program: [rust-vt-selection/src/main.rs](/example/rust-vt-selection/src/main.rs)

### Working with a selection

These all take a selection whose references are valid for the terminal's active
screen. They have to come from that terminal and that screen, with no change to
the terminal since. We don't check, so a stale reference is on you.

| Function                              | What it does                                                                             |
| ------------------------------------- | ---------------------------------------------------------------------------------------- |
| [`Selection::adjust()`]   | Moves the selection's logical end, as keyboard selection does. The result is a snapshot. |
| [`Selection::order()`]    | Tells you which way the ends are ordered.                                                |
| [`Selection::to_ordered()`]  | Returns a new snapshot with the ends in the order you ask for.                           |
| [`Selection::contains()`] | Tests whether a point is inside, for rectangular selections too.                         |
| [`Selection::equals()`]    | Tests whether two selections have the same ends and shape.                               |

## Mouse gestures {#guide-selection-gestures}

A selection gesture is a state machine that turns pointer events into
selections. It handles click counts, dragging, and what each click count
selects. You feed it events and it hands back snapshots.

Create one [`Gesture`] per stream of pointer events with
[`Gesture::new()`]. It isn't bound to a terminal, so the functions
that need one take it as an argument. Then create a gesture event
for each event type you use with its corresponding `new` function,
(e.g. [`PressEvent::new()`]). The type is fixed, but the
object is reusable: change its options with the various setters
 and apply it with the corresponding `apply()` method (e.g.
[`PressEvent::apply()`]).

| Event type                                            | `apply` arguments | Optional                                |
| ----------------------------------------------------- | ------------------ | --------------------------------------- |
| [`PressEvent`]           | Ref                | The rest use their defaults.            |
| [`DragEvent`]            | Ref, geometry      | Position, rectangle, word boundaries.   |
| [`ReleaseEvent`]         | None               | Ref. Unset means it wasn't over a cell. |
| [`AutoscrollTickEvent`] | Viewport, geometry | Position, rectangle, word boundaries.   |
| [`DeepPressEvent`]      | None               | Word boundaries.                        |

A press only gets repeat clicks if you call [`PressEvent::set_time()`].
Without a time, only single-click behavior is available. By default a single
click selects by cell, a double click by word and a triple click by line.
Change that with [`PressEvent::set_behaviors()`], which can also
select command output.

Getting `Ok(None)` from `apply()` isn't an error. It means this event doesn't
produce a selection right now, as with a normal single press. A release never
produces one, and in fact always returns a `Ok(())` when successful. When you
do get a selection, it isn't installed anywhere. You decide whether to draw it,
copy it, or install it as the terminal's selection.

```rs
{{#include ../../../example/rust-vt-selection-gesture/src/main.rs:selection-gesture-main}}
```

Full program: [rust-vt-selection-gesture/src/main.rs](/example/rust-vt-selection-gesture/src/main.rs)

### Dragging past the edge

When a drag reaches the top or bottom of the surface, the gesture wants the
viewport to scroll. Read [`Gesture::autoscroll()`] after each drag. While it's
up or down, run a timer of your own that sends [`AutoscrollTickEvent`]
events. Each tick scrolls the viewport one row and continues the drag.
The interval is up to you, and about 15 milliseconds is a reasonable default.
Scroll faster by ticking faster. If a tick returns [`Autoscroll::None`],
the gesture has cancelled itself, so stop the timer.

### Rules for gestures

Calls that use a gesture aren't safe to run concurrently, and they have to be
serialized with changes to the terminal.

[`Gesture::reset()`] cancels the current click sequence and
releases the gesture's tracked state without freeing it.

## Copying it out {#guide-selection-copying}

A selection names cells. To get the text, use a [formatter](Formatter) with
[`Formatter::set_selection`]. For the same text Ghostty puts on the
clipboard, use plain output with [`Formatter::set_unwrap`] and
[`Formatter::set_trim`] both true:

```rs
use libghostty_vt::{fmt::{Format, Formatter}};

let Some(selection) = terminal.selection()? else {
    return;
};

let mut formatter = Formatter::new(&terminal)?;
formatter
    .set_format(Format::Plain)?
    .set_unwrap(true)?
    .set_trim(true)?
    .set_selection(Some(selection))?;

let buf = formatter.format_alloc(None);
// Put buf on the clipboard.
```

The formatter keeps its own copy of the selection, but that copy is still only
valid until the next change to the terminal. After a change, set the selection
again before you format, or clear it.
