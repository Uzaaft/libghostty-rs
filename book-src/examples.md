# Examples {#example-programs}

Every example is a complete program you can build and run. Each one lives in its
own directory under `example/`, with a README that says how to run it. Most of
them are one `zig build run`. CI compiles all Rust examples, so they can't fall
behind the API.

If you want to see everything working together, start with [ghostling-rs](/example/ghostling_rs).
It's a minimal but complete terminal built on libghostty-vt, written in Rust.

If you only want to read a terminal's screen, start with
[rust-vt-stream](/example/rust-vt-stream/src/main.rs).

## Start Here

| Example                                         | What it does                                                                                                                                                                         |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [rust-vt-hello](/example/rust-vt-hello/src/main.rs)         | The smallest complete program: create a terminal, write text with a color sequence in it, and print the screen. This is the program on the main page.                                |
| [rust-vt-stream](/example/rust-vt-stream/src/main.rs)       | Feed VT sequences into a terminal, read the final screen back as plain text, and draw it with a render state. Start here for read-only tools like replay viewers and CI log viewers. |
| [rust-vt-formatter](/example/rust-vt-formatter/src/main.rs)  | Write text and cursor movement into a terminal, then stream the screen to stdout as plain text through a GhosttyWriter.                                                              |

## Terminal

| Example                                                         | What it does                                                                                                                                                                                                               | Guide                              |
| --------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------- |
| [rust-vt-colors](/example/rust-vt-colors/src/main.rs)                        | Set the default colors and palette, then read effective and default values as an OSC 10 override layers on top.                                                                                                            | [Colors](guides/colors.md#guide-colors)          |
| [rust-vt-effects](/example/rust-vt-effects/src/main.rs)                      | Register callbacks for what a program asks of the terminal: pty replies, the bell, title changes, OSC 52 clipboard reads and writes, and unknown sequences. You need these as soon as a real program runs in the terminal. | [Effects](guides/effects.md#guide-effects)        |
| [rust-vt-compression](/example/rust-vt-compression/src/main.rs)              | Compress scrollback while the terminal is idle.                                                                                                                                                                            | [Scrollback](guides/scrollback.md#guide-scrollback)  |
| [rust-vt-snapshot](/example/rust-vt-snapshot/src/main.rs)                    | Encode a terminal's full state and restore it three ways: in one shot, with scrollback kept compressed, and page by page. The page by page path gives you a terminal you can draw before the old history arrives.          | [Snapshots](guides/snapshots.md#guide-snapshots)    |
| [rust-vt-grid-traverse](/example/rust-vt-grid-traverse/src/main.rs)          | Walk the whole grid cell by cell with grid refs, reading codepoints, row wrap state and styles.                                                                                                                            | [The Grid](guides/grid.md#guide-grid)          |
| [rust-vt-grid-ref-tracked](/example/rust-vt-grid-ref-tracked/src/main.rs)    | Keep a reference to one cell while the terminal scrolls under it.                                                                                                                                                          | [The Grid](guides/grid.md#guide-grid)          |
| [rust-vt-render](/example/rust-vt-render/src/main.rs)                        | Draw a frame from a render state: check what's dirty, walk the rows and cells, and read styles, colors, the cursor and the selection. Use this when you render the terminal yourself.                                      | [Rendering](guides/rendering.md#guide-rendering)    |
| [rust-vt-search](/example/rust-vt-search/src/main.rs)                       | Search the terminal for a string, step through the matches like a find bar, and read the viewport matches to draw highlights.                                                                                              | [Search](guides/search.md#guide-search)         |
| [rust-vt-selection](/example/rust-vt-selection/src/main.rs)                  | Select a word, a line, a command's output or the whole screen, then format each selection as text. The line and output selections use OSC 133 prompt markers.                                                              | [Selection](guides/selection.md#guide-selection)    |
| [rust-vt-selection-gesture](/example/rust-vt-selection-gesture/src/main.rs)  | Feed press, drag, release and deep press events to ghostty_selection_gesture_event() and get back selections you can show or copy.                                                                                         | [Selection](guides/selection.md#guide-selection)    |
| [rust-vt-kitty-graphics](/example/rust-vt-kitty-graphics/src/main.rs)        | Accept images sent with the Kitty Graphics Protocol and find out what to draw. It installs a stand-in PNG decoder, reads placements from the render state, and plays an animation.                                         | [Images](guides/images.md#guide-images)          |

## Input

| Example                                               | What it does                                                                                                                                                                                                                            | Guide                    |
| ----------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| [rust-vt-encode-key](/example/rust-vt-encode-key/src/main.rs)      | Encode Ctrl+C as an escape sequence with the Kitty keyboard protocol.                                                                                                                                                                   | [Input](guides/input.md#guide-input)  |
| [rust-vt-encode-mouse](/example/rust-vt-encode-mouse/src/main.rs)  | Encode a left button press at a pixel position as an SGR mouse sequence.                                                                                                                                                                | [Input](guides/input.md#guide-input)  |
| [rust-vt-encode-focus](/example/rust-vt-encode-focus/src/main.rs)  | Encode a focus gained event as CSI I, for programs that turn on focus reporting (mode 1004).                                                                                                                                            | [Input](guides/input.md#guide-input)  |
| [rust-vt-paste](/example/rust-vt-paste/src/main.rs)                | Paste into a terminal with ghostty_terminal_paste(): plain text, bracketed paste (mode 2004), the unsafe-paste confirmation, and Kitty paste events (mode 5522). It also checks paste safety and encodes paste data without a terminal. | [Input](guides/input.md#guide-input)  |

## Building Blocks

| Example                                               | What it does                                                                                                                          | Guide                      |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | -------------------------- |
| [rust-vt-osc](/example/rust-vt-osc/src/main.rs)                    | Parse an OSC sequence and read the window title out of it.                                                                            |                            |
| [rust-vt-sgr](/example/rust-vt-sgr/src/main.rs)                    | Parse SGR sequences into text attributes, including a real one from Kakoune with a curly underline and RGB colors.                    |                            |
| [rust-vt-modes](/example/rust-vt-modes/src/main.rs)                | Pack DEC and ANSI mode numbers into a GhosttyMode and encode a DECRPM report.                                                         |                            |
| [rust-vt-color-scheme](/example/rust-vt-color-scheme/src/main.rs)  | Encode a color scheme report, the reply to a CSI ? 996 n query.                                                                       | [Colors](guides/colors.md#guide-colors)  |
| [rust-vt-size-report](/example/rust-vt-size-report/src/main.rs)    | Encode an in-band size report (mode 2048) from the grid and cell size.                                                                |                            |
| [rust-vt-allocator](/example/rust-vt-allocator/src/main.rs)        | Run a terminal on your own allocator, built on `malloc` and `free`. It counts live bytes to check that the terminal frees everything. |                            |
| [rust-vt-build-info](/example/rust-vt-build-info/src/main.rs)      | Query build options like SIMD and Kitty graphics, then check that the loaded library matches the headers you compiled against.        |                            |

## Other Languages

C language examples can be found in the [original documentation](https://lib.ghostty.org/vt/example-programs.html). Examples written in other languages can be found there as well, under the
"Other Languages" section.
