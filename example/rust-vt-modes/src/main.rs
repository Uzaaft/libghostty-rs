use libghostty_vt::{
    error::Error,
    mode::{Mode, ReportState},
};

// ANCHOR: modes-decrpm
fn decrpm_example() -> Result<(), Error> {
    // Encode a report that DEC mode 25 (cursor visible) is set
    let mut buf = [0u8; 32];
    let written = Mode::CURSOR_VISIBLE.encode(ReportState::Set, &mut buf)?;

    // Prints: ESC[?25;1$y
    println!(
        "Encoded {written} bytes: {}",
        buf[0..written].escape_ascii()
    );
    Ok(())
}
// ANCHOR_END: modes-decrpm

fn main() -> Result<(), Error> {
    decrpm_example()?;
    Ok(())
}
