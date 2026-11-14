use libghostty_vt::{
    error::Error,
    grid_ref::TrackedGridRef,
    point::{Coordinate, Point, Space},
    Terminal,
};

// ANCHOR: grid-ref-tracked
fn codepoint_at_tracked_ref(
    terminal: &Terminal<'_, '_>,
    tracked: &TrackedGridRef,
) -> Result<char, Error> {
    let snapshot = tracked
        .snapshot(terminal)?
        .expect("tracked grid ref should be active");

    let cell = snapshot.cell()?;
    assert!(cell.has_text()?);

    cell.codepoint()
}

fn main() -> Result<(), Error> {
    let mut terminal = Terminal::new(8, 3)?;

    terminal.vt_write(b"alpha\r\nbravo\r\ncharlie");

    let alpha = Point::Active(Coordinate { x: 0, y: 0 });
    let mut tracked = terminal.track_grid_ref(alpha)?;

    // Writing another line scrolls the original "alpha" row into scrollback.
    // The tracked ref still follows the same cell.
    terminal.vt_write(b"\r\ndelta");

    assert!(tracked.has_value());
    println!(
        "tracked codepoint after scroll: {}",
        codepoint_at_tracked_ref(&terminal, &tracked)?
    );

    let screen = tracked
        .point(Space::Screen)?
        .expect("tracked point should be on screen");
    println!("tracked screen point: {},{}", screen.x, screen.y);

    // Resetting the terminal discards the old grid contents. The tracked
    // handle remains valid, but no longer has a meaningful location.
    terminal.reset();
    assert!(!tracked.has_value());
    assert!(tracked.snapshot(&terminal)?.is_none());

    // The same handle can be moved to a new point after it loses its value.
    terminal.vt_write(b"echo");

    let echo = Point::Active(Coordinate { x: 0, y: 0 });
    tracked.set(&mut terminal, echo)?;
    assert!(tracked.has_value());
    println!(
        "tracked codepoint after reset/set: {}",
        codepoint_at_tracked_ref(&terminal, &tracked)?
    );

    Ok(())
}
// ANCHOR_END: grid-ref-tracked
