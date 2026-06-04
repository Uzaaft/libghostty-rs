use std::{error::Error, io};

use libghostty_vt::{
    Terminal, TerminalOptions,
    screen::TrackedGridRef,
    terminal::{Point, PointCoordinate, PointSpace},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn codepoint_at_tracked_ref(terminal: &Terminal<'_, '_>, tracked: &TrackedGridRef) -> Result<char> {
    let snapshot = tracked
        .snapshot(terminal)?
        .ok_or_else(|| io::Error::other("tracked grid reference has no value"))?;
    let cell = snapshot.cell()?;

    assert!(cell.has_text()?, "tracked cell should contain text");

    let codepoint = cell.codepoint()?;
    let character = char::from_u32(codepoint)
        .ok_or_else(|| io::Error::other("cell codepoint is not a Unicode scalar value"))?;

    Ok(character)
}

fn main() -> Result<()> {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 8,
        rows: 3,
        max_scrollback: 100,
    })?;

    terminal.vt_write(b"alpha\r\nbravo\r\ncharlie");

    let alpha = Point::Active(PointCoordinate { x: 0, y: 0 });
    let mut tracked = terminal.track_grid_ref(alpha)?;

    // Writing another line scrolls the original "alpha" row into scrollback.
    // The tracked ref still follows the same cell.
    terminal.vt_write(b"\r\ndelta");

    assert!(tracked.has_value(), "tracked ref should survive scrolling");
    println!(
        "tracked codepoint after scroll: {}",
        codepoint_at_tracked_ref(&terminal, &tracked)?
    );

    let screen = tracked
        .point(PointSpace::Screen)?
        .ok_or_else(|| io::Error::other("tracked grid reference has no screen point"))?;
    println!("tracked screen point: {},{}", screen.x, screen.y);

    // Resetting the terminal discards the old grid contents. The tracked
    // handle remains valid, but no longer has a meaningful location.
    terminal.reset();
    assert!(
        !tracked.has_value(),
        "tracked ref should lose its value after reset"
    );
    assert!(
        tracked.snapshot(&terminal)?.is_none(),
        "snapshot should report no value after reset"
    );

    // The same handle can be moved to a new point after it loses its value.
    terminal.vt_write(b"echo");

    let echo = Point::Active(PointCoordinate { x: 0, y: 0 });
    tracked.set(&mut terminal, echo)?;
    assert!(
        tracked.has_value(),
        "tracked ref should have a value after set"
    );
    println!(
        "tracked codepoint after reset/set: {}",
        codepoint_at_tracked_ref(&terminal, &tracked)?
    );

    Ok(())
}
