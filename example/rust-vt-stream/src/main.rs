use libghostty_vt::{
    fmt,
    render::{CellIterator, RenderState, RowIterator},
    Terminal,
};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // ANCHOR: vt-stream-init
    let mut terminal = Terminal::new(80, 24)?;
    // ANCHOR_END: vt-stream-init

    // ANCHOR: vt-stream-write
    // Feed VT data into the terminal
    terminal.vt_write(b"Hello, World!\r\n");

    // ANSI color codes: ESC[1;32m = bold green, ESC[0m = reset
    terminal.vt_write(b"\x1b[1;32mGreen Text\x1b[0m\r\n");

    // Cursor positioning: ESC[1;1H = move to row 1, column 1
    terminal.vt_write(b"\x1b[1;1HTop-left corner\r\n");

    // Cursor movement: ESC[5B = move down 5 lines
    terminal.vt_write(b"\x1b[5B");
    terminal.vt_write(b"Moved down!\r\n");

    // Erase line: ESC[2K = clear entire line
    terminal.vt_write(b"\x1b[2K");
    terminal.vt_write(b"New content\r\n");

    // Multiple lines
    terminal.vt_write(b"Line A\r\nLine B\r\nLine C\r\n");
    // ANCHOR_END: vt-stream-write

    // ANCHOR: vt-stream-read
    // Get the final terminal state as a plain string using the formatter
    let mut formatter = fmt::Formatter::new(&terminal)?;
    formatter.set_format(fmt::Format::Plain)?.set_trim(true)?;

    let buf = formatter.format_alloc(None)?;
    println!("{}", buf.escape_ascii());
    // ANCHOR_END: vt-stream-read

    // ANCHOR: vt-stream-render
    // Create these once and reuse them for every frame.
    let mut render_state = RenderState::new()?;
    let mut rows = RowIterator::new()?;
    let mut cells = CellIterator::new()?;

    // Capture the frame. This is the only step that reads the terminal.
    let snapshot = render_state.update(&terminal)?;

    // Visit only the rows that changed. In the first frame, that's all of them.
    let mut row_iter = rows.update(&snapshot)?;
    while let Some((_y, row)) = row_iter.next_dirty() {
        let mut cell_iter = cells.update(&row)?;

        while let Some(cell) = cell_iter.next() {
            let style = cell.style()?;

            let mut graphemes = String::new();
            cell.graphemes_utf8(&mut graphemes)?;

            // A real renderer draws the cell here. We print it, with its bold
            // style, which the plain text from the formatter didn't have.
            if graphemes.is_empty() {
                print!(" ");
                continue;
            }

            if style.bold {
                print!("\x1b[1m");
            }
            print!("{}", graphemes);
            if style.bold {
                print!("\x1b[0m");
            }
        }
        println!();
    }

    // The frame is drawn, so the next one only reports new changes.
    snapshot.clean()?;
    // ANCHOR_END: vt-stream-render

    Ok(())
}
