//! Register callbacks for what a program asks of the terminal: pty replies,
//! the bell, title changes, OSC 52 clipboard reads and writes, and unknown
//! sequences. You need these as soon as a real program runs in the terminal.
use libghostty_vt::{
    effect::{ClipboardReplyContent, UnknownSequence},
    osc::Terminator,
    Terminal,
};
use std::{cell::Cell, error::Error};

fn main() -> Result<(), Box<dyn Error>> {
    // ANCHOR: effects-state-lifetime
    // Specific to the Rust bindings: any state borrowed by the callbacks
    // mutably is held mutably for the entire lifetime of the terminal,
    // which means that it cannot be accessed by other parties, including
    // other callbacks. They must therefore also outlive the terminal.
    //
    // To prevent this, use a type with _interior mutability_ which allows
    // the value inside to be safely mutated by an immutable, shared reference.
    // In this case, `usize` is a simple `Copy` type, so we can use `Cell`s
    // here. Non-copy types need to use `RefCell`s instead.
    let bell_count = Cell::new(0usize);
    // ANCHOR_END: effects-state-lifetime

    // Create a terminal
    let mut terminal = Terminal::new(80, 24)?;

    // Register effect callbacks
    // ANCHOR: effects-write-pty
    terminal.on_pty_write(|_term, data| {
        println!(
            "  pty_write ({} bytes): {}",
            data.len(),
            data.escape_ascii()
        );
    })?;
    // ANCHOR_END: effects-write-pty

    // ANCHOR: effects-bell
    terminal.on_bell({
        // Explicitly borrow the bell count, or otherwise `move`
        // will attempt to capture the entire `Cell` and cause a
        // compiler error
        let bell_count = &bell_count;
        move |_term| {
            bell_count.update(|v| v + 1);
            println!("  bell! (count={})", bell_count.get());
        }
    })?;
    // ANCHOR_END: effects-bell

    // ANCHOR: effects-title-changed
    terminal.on_title_changed(|term| {
        // The new title is already set, so you can read it here with
        // `Terminal::title`. This example reads the cursor position
        // to show that the terminal can be queried from a callback.
        let col = term.cursor_x().unwrap();
        println!("  title changed (cursor at col {col})");
    })?;
    // ANCHOR_END: effects-title-changed

    // ANCHOR: effects-clipboard-write
    terminal.on_clipboard_write(|_term, write| {
        // The write is synchronous: a real embedder would ask the user for
        // permission here (unless `write.granted()`) and the VT stream waits until
        // this callback returns. The replied result is sent to the program
        // (OSC 5522) through the on_pty_write callback.

        let contents = write.contents();
        println!(
            "  clipboard write (location={:?}, contents={})",
            write.location(),
            contents.len()
        );

        if contents.len() == 0 {
            println!("    clear");
        }

        for content in contents {
            print!("    ");
            if !content.mime.is_empty() {
                print!("{}", content.mime.escape_ascii());
            }
            print!(" ({} bytes): ", content.data.len());
            if !content.data.is_empty() {
                print!("{}", content.data.escape_ascii());
            }
            println!();
        }

        write.reply(Ok(()), false);
    })?;
    // ANCHOR_END: effects-clipboard-write

    // ANCHOR: effects-clipboard-read
    terminal.on_clipboard_read(|_term, read| {
        // The read is synchronous: a real embedder would ask the user for
        // permission here (unless read->granted) and the VT stream waits until
        // this callback returns. The reply is sent to the program through the
        // on_pty_write callback.

        let mimes = read.mimes();
        println!(
            "  clipboard read (location={:?}, mimes={})",
            read.location(),
            mimes.len()
        );

        for mime in mimes {
            println!("    {}", mime.escape_ascii());
        }

        // Reply with every requested representation we have.
        // This example only has text.
        read.reply(
            Ok(&[ClipboardReplyContent::new(
                b"text/plain",
                b"Hello from the clipboard",
            )]),
            &[],
            false,
        );
    })?;
    // ANCHOR_END: effects-clipboard-read

    // ANCHOR: effects-unknown-sequence
    terminal
        .on_unknown_sequence(|_term, seq| match seq {
            UnknownSequence::Apc {
                content, truncated, ..
            } => {
                println!(
                    "  unknown APC (truncated={}, content={} bytes): {}",
                    if truncated { "yes" } else { "no" },
                    content.len(),
                    content.escape_ascii(),
                );
            }
            UnknownSequence::Osc {
                content,
                truncated,
                terminator,
                ..
            } => {
                // The content starts with the OSC number, for example
                // "7400;status=busy". A real application would check for the
                // numbers it implements and ignore the rest. If it replies, it should
                // end the reply with the same terminator the program used.
                println!(
                    "  unknown OSC (truncated={}, terminator={}, content={} bytes): {}",
                    if truncated { "yes" } else { "no" },
                    match terminator {
                        Terminator::Bel => "BEL",
                        Terminator::St => "ST",
                        _ => "unknown",
                    },
                    content.len(),
                    content.escape_ascii(),
                );
            }
            _ => {}
        })?
        // Unknown sequence capture is independently bounded and disabled by
        // default. This limit will apply to every supported unknown sequence type.
        .set_unknown_max_bytes(256)?;
    // ANCHOR_END: effects-unknown-sequence

    // ANCHOR: effects-register
    // Feed VT data that triggers effects:

    // 1. Bell (BEL = 0x07)
    println!("Sending BEL:");
    terminal.vt_write(b"\x07");

    // 2. Title change (OSC 2 ; <title> ST)
    println!("Sending title change:");
    terminal.vt_write(b"\x1B]2;Hello Effects\x1B\\");

    // 3. Device status report (DECRQM for wraparound mode ?7)
    //    triggers write_pty with the response
    println!("Sending DECRQM query:");
    terminal.vt_write(b"\x1B[?7$p");

    // 4. Clipboard write (OSC 52 ; c ; <base64 data> ST)
    println!("Sending clipboard write:");
    terminal.vt_write(b"\x1B]52;c;SGVsbG8gY2xpcGJvYXJk\x1B\\");

    // 5. Clipboard read (OSC 52 ; c ; ? ST)
    println!("Sending clipboard read:");
    terminal.vt_write(b"\x1B]52;c;?\x1B\\");

    // 6. Unsupported APC sequence
    println!("Sending unknown APC:");
    terminal.vt_write(b"\x1B_private-command;payload\x1B\\");

    println!("Sending unknown OSC:");
    terminal.vt_write(b"\x1B]7400;status=busy\x07");

    // 7. Another bell to show the counter increments
    println!("Sending another BEL:");
    terminal.vt_write(b"\x07");

    println!("Total bells: {}", bell_count.get());
    // ANCHOR_END: effects-register
    Ok(())
}
