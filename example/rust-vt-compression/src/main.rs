//! Compress scrollback while the terminal is idle.
#![allow(unused_assignments)]
use libghostty_vt::{
    error::Error,
    terminal::{CompressionMode, CompressionResult, Terminal},
};

// ANCHOR: compression-idle-step
/// Perform one step after the application's idle timer fires. Returning true
/// asks the application to schedule another step while the terminal is idle.
fn compression_idle_step(terminal: &mut Terminal<'_, '_>) -> Result<bool, Error> {
    match terminal.compress(CompressionMode::Incremental)? {
        CompressionResult::Pending => Ok(true),
        CompressionResult::Unsupported | CompressionResult::Complete => Ok(false),
        _ => unreachable!(),
    }
}
// ANCHOR_END: compression-idle-step

// ANCHOR: memory-usage
/// Return the bytes of memory a terminal currently holds. A server hosting
/// many terminals can compare this figure across terminals to decide which
/// one to compress or shut down first.
fn terminal_resident_bytes(terminal: &Terminal<'_, '_>) -> Result<u64, Error> {
    let usage = terminal.memory_usage()?;

    // Count both screens and any images. The alternate screen reports zero
    // until a program such as a full-screen editor first switches to it.
    Ok(usage.primary_resident_bytes
        + usage.primary_image_bytes
        + usage.alternate_resident_bytes
        + usage.alternate_image_bytes)
}
// ANCHOR_END: memory-usage

fn main() -> Result<(), Error> {
    let mut terminal = Terminal::new(80, 24)?;
    terminal.set_scrollback_max_bytes(Some(10 * 1024 * 1024))?;

    // ANCHOR: compression-activity
    let mut activity = terminal.compression_activity()?;

    // Terminal mutations may change the token. When it changes, restart the
    // application's idle timer rather than compressing on the output path.
    for _ in 0..4000 {
        terminal.vt_write(b"repeated and compressible terminal history\r\n");
    }

    let new_activity = terminal.compression_activity()?;
    if activity != new_activity {
        activity = new_activity;
        // Restart the application's compression idle timer here.
    }
    // ANCHOR_END: compression-activity

    let before = terminal_resident_bytes(&terminal)?;
    // Simulate the idle timer and its short pending-work continuations.
    while compression_idle_step(&mut terminal)? {}
    let after = terminal_resident_bytes(&terminal)?;

    println!("resident memory: {before} bytes before compression, {after} after");
    Ok(())
}
