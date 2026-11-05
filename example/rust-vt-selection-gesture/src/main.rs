use libghostty_vt::{
    Terminal,
    error::Error,
    fmt::{Format, Formatter},
    point::{Coordinate, Point},
    selection::{
        Selection,
        gesture::{self, Gesture},
    },
};
use std::io::Write;

// ANCHOR: selection-gesture-main
fn print_selection<'t>(
    terminal: &'t Terminal<'_, '_>,
    label: &str,
    selection: Selection<'t>,
) -> Result<(), Error> {
    let mut formatter = Formatter::new(terminal)?;
    formatter
        .set_format(Format::Plain)?
        .set_trim(true)?
        .set_selection(Some(selection))?;

    let buf = formatter.format_alloc(None)?;

    let mut stdout = std::io::stdout();
    _ = write!(stdout, "{label}: ");
    _ = stdout.write_all(&buf);
    _ = writeln!(stdout);
    Ok(())
}

fn main() -> Result<(), Error> {
    let mut terminal = Terminal::new(20, 4)?;

    terminal.vt_write(b"hello world\r\nsecond line");

    let mut gesture = Gesture::new(&terminal)?;
    let mut press = gesture::PressEvent::new()?;
    let mut drag = gesture::DragEvent::new()?;
    let mut release = gesture::ReleaseEvent::new()?;
    let mut deep_press = gesture::DeepPressEvent::new()?;

    let geometry = gesture::Geometry {
        columns: 20,
        cell_width: 10,
        padding_left: 0,
        screen_height: 40,
    };

    // Press in the first cell. A normal single press records the click anchor but
    // doesn't produce a selection yet, so we discard the optional output.
    let selection = press.set_position(2.0, 8.0)?.apply(
        &mut gesture,
        &terminal,
        Point::Active(Coordinate { x: 0, y: 0 }),
    )?;
    assert!(selection.is_none());

    // Drag across "hello". The drag event returns a selection snapshot that the
    // embedder can apply to its UI, copy, or format immediately.
    let selection = drag
        .set_position(46.0, 8.0)?
        .apply(
            &mut gesture,
            &terminal,
            Point::Active(Coordinate { x: 4, y: 0 }),
            geometry,
        )?
        .expect("'hello' should be selected");
    print_selection(&terminal, "drag", selection)?;

    // Release updates gesture state but never produces a selection.
    release.apply(&mut gesture, &terminal, None)?;

    println!("dragged: {}", gesture.dragged(&terminal)?);

    // Deep press uses the active click anchor to select the surrounding word.
    gesture.reset(&terminal)?;

    let selection = press.apply(
        &mut gesture,
        &terminal,
        Point::Active(Coordinate { x: 6, y: 0 }),
    )?;
    assert!(selection.is_none());

    let selection = deep_press
        .apply(&mut gesture, &terminal)?
        .expect("'world' should be selected");
    print_selection(&terminal, "deep press", selection)?;

    Ok(())
}
// // ANCHOR: selection-gesture-main
