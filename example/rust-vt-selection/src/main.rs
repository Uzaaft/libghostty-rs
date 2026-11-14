use libghostty_vt::{
    error::Error,
    fmt::{Format, Formatter},
    grid_ref::GridRef,
    point::{Coordinate, Point},
    selection::Selection,
    Terminal,
};
use std::io::Write;

// ANCHOR: selection-main
fn ref_at<'t>(terminal: &'t Terminal<'_, '_>, x: u16, y: u32) -> GridRef<'t> {
    terminal
        .grid_ref(Point::Active(Coordinate { x, y }))
        .expect("ref should exist")
}

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
    let mut terminal = Terminal::new(80, 8)?;

    // A realistic shell transcript with OSC 133 semantic prompt markers.
    // Ghostty uses these markers to distinguish prompt/input from command
    // output for semantic line and output selections.
    terminal.vt_write(b"\x1B]133;A\x07$ "); // Prompt starts: "$ "
    terminal.vt_write(b"\x1B]133;B\x07git status"); // Input starts: "git status"
    terminal.vt_write(b"\x1B]133;C\x07\r\n"); // Output starts after Enter
    terminal.vt_write(b"On branch main\r\n");
    terminal.vt_write(b"nothing to commit, working tree clean");

    // Double-click style word selection under the cursor.
    let grid_ref = ref_at(&terminal, 6, 0); // the "status" in "git status"
    let selection = terminal
        .select_word(grid_ref)
        .finish()?
        .expect("word should be selected");
    print_selection(&terminal, "word", selection)?;

    // ANCHOR: selection-word-between
    // Double-click-and-drag style selection. Suppose the user double-clicks
    // "git" and drags to "status". The pointer may pass over whitespace, so
    // select the nearest word between the original click and current drag point
    // in both directions, then combine the outer word bounds.
    let click_ref = ref_at(&terminal, 2, 0); // the "git" in "git status"
    let drag_ref = ref_at(&terminal, 6, 0); // the "status" in "git status"

    let start_word = terminal
        .select_word_between(click_ref, drag_ref)
        .finish()?
        .expect("word should be selected");
    let end_word = terminal
        .select_word_between(drag_ref, click_ref)
        .finish()?
        .expect("word should be selected");

    let selection = terminal.select(start_word.start(), end_word.end(), false);
    print_selection(&terminal, "double-click drag", selection)?;
    // ANCHOR_END: selection-word-between

    // Triple-click style line selection. With semantic prompt boundaries enabled,
    // this selects only the input area rather than the leading "$ " prompt.
    let grid_ref = ref_at(&terminal, 2, 0); // the "git status" input area.
    let selection = terminal
        .select_line(grid_ref)
        .with_semantic_prompt_boundary(true)
        .finish()?
        .expect("line should be selected");
    print_selection(&terminal, "line", selection)?;

    // Select exactly the command output for the command under the cursor.
    let output_ref = ref_at(&terminal, 0, 1);
    let selection = terminal
        .select_output(output_ref)?
        .expect("output should be selected");
    print_selection(&terminal, "output", selection)?;

    // Select all visible content.
    let selection = terminal.select_all()?.expect("all should be selected");
    print_selection(&terminal, "all", selection)?;

    Ok(())
}
// ANCHOR_END: selection-main
