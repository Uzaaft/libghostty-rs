//! Draw a frame from a render state: check what's dirty, walk the rows
//! and cells, and read styles, colors, the cursor and the selection.
//! Use this when you render the terminal yourself.

use libghostty_vt::{
    Terminal,
    error::Error,
    point::{Coordinate, Point},
    render::{CellIterator, CursorVisualStyle, Dirty, RenderState, RowIterator},
    selection::Selection,
    style::Underline,
};

fn main() -> Result<(), Error> {
    // ANCHOR: render-state-update
    // Create a terminal and render state, then update the render state
    // from the terminal. The render state captures a snapshot of everything
    // needed to draw a frame.
    let mut terminal = Terminal::new(40, 5)?;
    let mut render_state = RenderState::new()?;

    // Feed some styled content into the terminal.
    terminal.vt_write(b"Hello, \033[1;32mworld\033[0m!\r\n"); // bold green "world"
    terminal.vt_write(b"\033[4munderlined\033[0m text\r\n"); // underlined text
    terminal.vt_write(b"\033[38;2;255;128;0morange\033[0m\r\n"); // 24-bit orange fg

    // Select "underlined" on the second row. Render state exposes this
    // later as a row-local selected cell range.
    let selection_start = terminal.grid_ref(Point::Active(Coordinate { x: 0, y: 1 }))?;
    let selection_end = terminal.grid_ref(Point::Active(Coordinate { x: 9, y: 1 }))?;
    let selection = terminal.select(selection_start, selection_end, false);
    terminal.set_selection(Some(selection))?;

    let snapshot = render_state.update(&terminal)?;
    // ANCHOR_END: render-state-update

    // ANCHOR: render-dirty-check
    // Check the global dirty state to decide how much work the renderer
    // needs to do. After rendering, reset it to false.
    match snapshot.dirty()? {
        Dirty::Clean => println!("Frame is clean, nothing to draw."),
        Dirty::Partial => println!("Partial redraw needed."),
        Dirty::Full => println!("Full redraw needed."),
    }
    // ANCHOR_END: render-dirty-check

    // ANCHOR: render-colors
    // Retrieve colors (background, foreground, palette) from the render
    // state. These are needed to resolve palette-indexed cell colors.
    let colors = snapshot.colors()?;
    println!(
        "Background: #{:02x}{:02x}{:02x}",
        colors.background.r, colors.background.g, colors.background.b
    );
    println!(
        "Foreground: #{:02x}{:02x}{:02x}",
        colors.foreground.r, colors.foreground.g, colors.foreground.b
    );
    // ANCHOR_END: render-colors

    // ANCHOR: render-cursor
    // Read all cursor state in one call.
    let cursor = snapshot.cursor()?;
    if cursor.visible
        && let Some(viewport) = cursor.viewport
    {
        let style_name = match cursor.visual_style {
            CursorVisualStyle::Bar => "bar",
            CursorVisualStyle::Block => "block",
            CursorVisualStyle::Underline => "underline",
            CursorVisualStyle::BlockHollow => "hollow",
            _ => "unknown",
        };
        println!(
            "Cursor at ({}, {}), style: {}",
            viewport.x, viewport.y, style_name
        );
    }
    // ANCHOR_END: render-cursor

    // ANCHOR: render-row-iterate
    // Iterate rows via the row iterator. For each dirty row, iterate its
    // cells, read codepoints/graphemes and styles, and emit ANSI-colored
    // output as a simple "renderer".
    let mut rows = RowIterator::new()?;
    let mut cells = CellIterator::new()?;

    let mut row_it = rows.update(&snapshot)?;
    while let Some((y, row)) = row_it.next_dirty() {
        print!("Row {y} [dirty]: ");

        // Query the row-local selection range. Rows without a selection return
        // `None`; selected rows return inclusive start/end columns.
        if let Some(selection) = row.selection()? {
            print!("selection={}..{}", selection.start_x, selection.end_x);
        }

        // Get cells for this row (reuses the same cells handle).
        let mut cell_it = cells.update(&row)?;
        while let Some(cell) = cell_it.next() {
            // Get the graphemes of this cell.
            let graphemes = cell.graphemes()?;
            if graphemes.is_empty() {
                print!(" ");
                continue;
            }

            // Read the style for this cell. Returns the default style for
            // cells that have no explicit styling.
            let style = cell.style()?;

            // Resolve foreground color for this cell.
            let fg = cell.fg_color()?.unwrap_or(colors.foreground);

            // Emit ANSI true-color escape for the foreground.
            print!("\x1b[38;2;{};{};{}m", fg.r, fg.g, fg.b);

            if style.bold {
                print!("\x1b[1m");
            }
            if style.underline != Underline::None {
                print!("\x1b[4m");
            }

            // Print the read grapheme codepoints.
            // In a real renderer you would reuse a grapheme buffer for this step.
            print!("{}", String::from_iter(graphemes));
            print!("\x1b[0m"); // Reset style after each cell.
        }

        println!()
    }
    // ANCHOR_END: render-row-iterate

    // ANCHOR: render-dirty-reset
    // After successfully rendering the complete frame, clear both the global
    // and per-row dirty state in one call.
    snapshot.clean()?;
    // ANCHOR_END: render-dirty-reset

    Ok(())
}
