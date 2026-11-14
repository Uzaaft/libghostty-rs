use libghostty_vt::{
    error::Error,
    point::{Coordinate, Point},
    Terminal,
};

// ANCHOR: grid-ref-traverse
fn main() -> Result<(), Error> {
    // Create a small terminal
    let mut terminal = Terminal::new(10, 3)?;

    // Write some content so the grid has interesting data
    terminal.vt_write(b"Hello!\r\n"); // Row 0: H e l l o !
    terminal.vt_write(b"World\r\n"); // Row 1: W o r l d
    terminal.vt_write(b"\033[1mBold"); // Row 2: B o l d (bold style)

    // Get terminal dimensions
    let cols = terminal.cols()?;
    let rows = terminal.rows()?;

    // Traverse the entire grid using grid refs
    for row in 0..rows {
        print!("Row {row}: ");
        for col in 0..cols {
            // Resolve the point to a grid reference
            let point = Point::Active(Coordinate { x: col, y: row });
            let grid_ref = terminal.grid_ref(point)?;

            // Read the cell from the grid ref
            let cell = grid_ref.cell()?;

            // Check if the cell has text
            if cell.has_text()? {
                let codepoint = cell.codepoint()?;
                print!("{codepoint}");
            } else {
                print!(".");
            }
        }

        // Also inspect the row for wrap state
        let point = Point::Active(Coordinate { x: 0, y: row });
        let grid_ref = terminal.grid_ref(point)?;
        let row = grid_ref.row()?;
        print!(" (wrap={}", row.wrap()?);

        // Check the style of the first cell with text
        let style = grid_ref.style()?;
        print!(", bold={}", style.bold);
    }

    Ok(())
}
// ANCHOR_END: grid-ref-traverse
