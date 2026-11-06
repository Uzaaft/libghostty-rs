# Rendering {#guide-rendering}

A render state is a copy of everything you need to draw one frame: the rows, the
cells, the cursor, and the colors.

Why a copy? Your renderer and your IO are usually on different threads. The
terminal changes every time the program writes a byte, and you can't draw from
something that is changing underneath you. You could hold a lock for the whole
frame, but then the program stalls while you draw.

With a render state you take the lock, update, and let go. Everything after that
reads only from the render state.

It's also built for being updated over and over from the same terminal. An
update only redoes what changed, and it tells you what changed so you can redraw
only that.

If you only want the screen as text, you don't need this page. Use the @ref
formatter instead. Images have their own guide, @ref guide-images.

## Drawing a frame {#guide-rendering-frame}

There are three steps:

1. Create an empty render state.
2. Update it from a terminal whenever you need a new frame. This returns a
   snapshot of the render state.
3. Read from the snapshot to draw.

@snippet c-vt-render/src/main.c render-state-update

Full program: @ref c-vt-render/src/main.c

Now read what you need. The colors come as one struct:

@snippet c-vt-render/src/main.c render-colors

Full program: @ref c-vt-render/src/main.c

So does the cursor:

@snippet c-vt-render/src/main.c render-cursor

Full program: @ref c-vt-render/src/main.c

Rows and cells have their own iterators. Row and cell data is only valid until
the next update.

@snippet c-vt-render/src/main.c render-row-iterate

Full program: @ref c-vt-render/src/main.c

## Two-phase updates {#guide-rendering-two-phase}

[`RenderState::update`] needs exclusive access to the terminal for the
whole call. If a renderer thread shares a lock with an IO thread, you can hold
that lock for less time by splitting the update in two.

[`RenderState::begin_update`] needs the terminal and returns a [`render::Update`]
object. [`render::Update::end`] finishes any work that was deferred, and it
only touches memory the render state owns. So lock, begin, unlock, and then end
while the IO thread goes back to modifying the terminal:

```rs
let update = {
  let terminal = terminal_mutex.lock().unwrap();
  render_state.begin_update(&terminal)
};
let snapshot = update.end();
```

Every begin needs an end before you read the render state, because it's
incomplete until then. [`RenderState::update`] is the same as calling the two
back to back.

## Dirty tracking {#guide-rendering-dirty}

Most frames change very little, so the render state tracks what changed and you
redraw only that.

There are two independent layers. A global state says if the frame is clean,
partially dirty or fully dirty. A flag on each row says which rows changed in a
partially dirty frame.

@snippet c-vt-render/src/main.c render-dirty-check

Full program: @ref c-vt-render/src/main.c

An update only ever sets dirty state, it never clears it. That's your job. Once
you've drawn a complete frame, [`Snapshot::clean`] clears both layers in one
call:

@snippet c-vt-render/src/main.c render-dirty-reset

Full program: @ref c-vt-render/src/main.c

If you only use part of a frame, clear what you used with
[`Snapshot::set_dirty`] and [`RowIteration::set_dirty`] instead.
Be careful here, because clearing one layer doesn't clear the other. Setting the
global state to false leaves every row flag as it was, so you have to manage
both.

## Render holds {#guide-rendering-holds}

A render hold is the time between a program asking the terminal to stop updating
the screen and letting it update again.

Programs use holds to avoid flicker. A full-screen program usually redraws in
several steps: clear, draw the text, move the cursor. If you draw halfway
through, the person using your terminal sees a broken frame. So the program
starts a hold, draws everything, and releases it. The screen should keep showing
the last finished frame the whole time and then switch to the new one all at
once.

Today the only way to start a hold is synchronized output (DEC private mode
2026, [`Mode::SYNC_OUTPUT`]). The callback is named for what you should do
rather than for that mode, so that other sources of holds can be added later.

[`RenderState::update`] doesn't check for any of this. It always captures
the terminal as it is right now, so a renderer that updates on every draw shows
half-drawn frames. To avoid that:

1. Set a [`Terminal::on_render_hold`] callback on the terminal.
2. When a hold begins, update the render state from inside the callback. At that
   moment the terminal holds exactly the frame the program wants left on screen.
   Nothing after the start of the hold has been processed yet, even if more
   bytes follow in the same [`Terminal::vt_write`].
3. Don't update the render state again until the hold ends. You can keep drawing
   it in the meantime, and it won't change.
4. End the hold yourself if it lasts too long.

You need the timeout. The terminal has no clock, so it never ends a hold on its
own, and a program that crashes or forgets to release its hold would freeze your
screen forever. One second is a common limit. When it expires, reset the mode
yourself and go back to updating normally. Setting the mode again during a hold
does nothing, so a program can't keep pushing your deadline back.

Why a callback? You could check [`Mode::SYNC_OUTPUT`] before each draw and
skip the update when it's set. That's simpler, but it has two problems. First,
the frame left on screen is whatever you happened to draw last, which can be
older than what the program intended or even half-drawn. Second, if the program
releases a hold and starts the next one between two of your draws, you never see
the mode turn off, and the finished frame in between is lost. A program that
draws continuously can then look frozen. Capturing the frame when each hold
begins avoids both.

You're free to ignore a hold whenever live content matters more, such as when
someone scrolls or starts a selection.

Like every callback, this one runs on the thread that called
[`Terminal::vt_write`]. If another thread draws the render state, protect
the update in the callback the same way you protect any other access to it.

## Overscan {#guide-rendering-overscan}

By default, a render state captures exactly the rows visible in the viewport.
That's all you need if you draw whole rows, so most programs can skip this
section.

Some renderers draw the grid shifted by a fraction of a row, usually to scroll
smoothly. While the grid is shifted, part of a row just outside the viewport
shows at one edge, and you need that row's content to draw it. Overscan captures
extra rows above and below the viewport for this.

Request it with [`RenderState::set_overscan`]. The request applies to every
update after you set it, and you should expect a full redraw on the update after
a change. The row iterator then visits the extra rows along with the viewport,
from top to bottom. [`RowIteration::viewport_y`] tells you where
each row belongs: rows above the viewport are negative, viewport rows are 0
through `snapshot.rows() - 1`, and rows below the viewport start at `snapshot.rows()`.

Extra rows are only captured when they exist. There's nothing above the first
line of scrollback, and there's nothing below the viewport while it's scrolled
to the bottom, which is the usual case. After an update, [`Snapshot::overscan`]
reports how many rows were actually captured on each side. Don't shift the grid
toward a side where nothing was captured.

Extra rows carry the same data as viewport rows, including cells, styles, dirty
flags and selection. The cursor is only reported when it's inside the viewport.

The render state doesn't store the scroll position. If you need it, read
[`Terminal::scrollbar`] or [`Terminal::viewport_active`] at the same time as you
call [`RenderState::begin_update`], while you have exclusive access to the
terminal. The values then describe the same moment as the render state.

This draws one frame of a smooth scroll. `offset_px` comes from your own scroll
animation. It's how far the grid is shifted up, from zero up to but not
including one row height.

```rs
// Once, when setting up the render state. One row below the viewport
// is enough to draw the partially visible row at the bottom edge.
render_state.set_overscan(Overscan { above: 0, below: 1 })?;

// Each frame.
let snapshot = render_state.update(&terminal)?;
let captured = snapshot.overscan()?;

// With no row below the viewport, there is nothing to scroll into.
if captured.below == 0 { offset_px = 0; }

let mut row_iter = rows.update(&snapshot)?;
while let Some(row) = row_iter.next() {
  let y = row.viewport_y()?;
  draw_row(row, y * cell_height - offset_px);
}
```

## Row identity {#guide-rendering-row-identity}

Every row has an id, [`RowIteration::id`], that stays with the row
as it moves. When the viewport scrolls by one row, each row shows up one
position higher or lower in the next update but keeps its id. Ids work with or
without overscan.

Ids let you keep expensive per-row work, such as shaped text or a prepared
texture, in your own cache keyed by id. You can reuse a cached entry when both
of these are true:

1. A row with the same id is in the new update.
2. That row's dirty flag isn't set.

The dirty flag is conservative. A row can be marked dirty even though its
content didn't change. For example, every row is currently marked dirty after
the viewport scrolls. Rebuilding a dirty row is always correct.

An id disappears when its row is no longer captured, is removed from scrollback,
or is changed in place by the terminal, for example when a program scrolls only
part of the screen. Ids are never reused, so an old id can never match a
different row. You can throw away cache entries for ids that no longer appear.

```rs
while let Some(row) = row_iter.next() {
  let id = row.id()?;

  // `cache` is your own map from row id to prepared row.
  let prepared = if row.dirty()? { None } else { cache.find(id) };
  let prepared = prepared.or_else(|| cache.prepare(id, row));
  draw_prepared_row(prepared);
}
```
