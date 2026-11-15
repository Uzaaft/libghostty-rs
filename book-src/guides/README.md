# Guides

A guide is a page you read top to bottom to learn how to do one thing. The API
Reference is where you look something up. If you know what you want to do but
not which functions do it, start here.

- [Effects](effects.md) - Answer the things a program asks of the terminal,
  such as the bell, title changes and queries.
- [Rendering](rendering.md) - Draw a terminal: copy a frame out from under the
  lock, redraw only what changed, and respect render holds.
- [Input](input.md) - Encode keys, mouse events, focus changes and pastes,
  and keep the encoders in step with the terminal's modes.
- [Colors](colors.md) - Set a theme, read colors back, parse color strings,
  and report light or dark mode.
- [Grid](grid.md) - Name a position in the grid, read its cells, and hold
  onto a cell as the terminal changes.
- [Selection](selection.md) - Select text, turn pointer gestures into selections,
  and copy them out.
- [Scrollback](scrollback.md) - Limit history, scroll through it, draw a
  scrollbar, and keep its memory down.
- [Search](search.md) - Find a string in the screen and scrollback, and drive
  the search from your event loop or a thread.
- [Images](images.md) - Accept the images programs send, decode PNGs, draw
  them, and play animations.
- [Snapshots](snapshots.md) - Save a terminal's full state, and restore it in one
  call or a piece at a time.
- [Custom Sequences](custom-sequences.md) - Handle sequences we don't implement, and
  stop a write at a string boundary.
