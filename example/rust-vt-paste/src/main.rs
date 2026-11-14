//! Paste into a terminal with `Terminal::paste`: plain text, bracketed paste
//! (mode 2004), the unsafe-paste confirmation, and Kitty paste events
//! (mode 5522). It also checks paste safety and encodes paste data without
//! a terminal.

use libghostty_vt::{effect, error::Error, paste, Terminal};
use std::{
    cell::RefCell,
    io::{self, Write},
};

// ANCHOR: terminal-paste
/// What the clipboard holds. A real embedder would keep a handle to the
/// pasteboard or its items here; the data is only produced on demand.
struct Clipboard {
    text: &'static str,
}

impl Clipboard {
    /// Produces the data of one representation when the terminal needs it.
    /// Only the text is ever read: the image is listed on a paste event
    /// but never requested, so a large image costs nothing to paste.
    /// Nothing written to the writer is retained, so the data can be
    /// streamed from anywhere in pieces of any size.
    fn read(&self, mime: &[u8], writer: &mut dyn Write) -> io::Result<()> {
        match mime {
            b"text/plain" => {
                // Stream the text in small pieces just to show that it works.
                for chunk in self.text.as_bytes().chunks(4) {
                    writer.write(chunk)?;
                }
                Ok(())
            }
            _ => Err(io::Error::other(
                "image read requested, which never happens",
            )),
        }
    }
}

// Paste whatever the clipboard holds. The terminal applies its own
// state: bracketed paste framing (mode 2004) or a Kitty paste event
// (mode 5522) instead of the text.
fn paste_clipboard(terminal: &mut Terminal<'_, '_>, text: &'static str) {
    let clipboard = Clipboard { text };

    let options = paste::Options::new()
        .with_location(effect::ClipboardLocation::Standard)
        .with_source(paste::Source::Clipboard);

    let mimes = &[
        // The first text representation is what a text paste writes.
        effect::ClipboardMime::new(b"text/plain"),
        // Listed on a paste event, never read.
        effect::ClipboardMime::new(b"image/png"),
    ];

    let mut result = terminal.paste(options, mimes, |mime, writer| clipboard.read(mime, writer));

    if let Err(Error::Rejected) = result {
        // The text could inject commands (e.g. a newline outside of a
        // bracketed paste). Nothing was written; ask, then retry.

        if !confirm_with_user() {
            return;
        }
        result = terminal.paste(options.with_allow_unsafe(true), mimes, |mime, writer| {
            clipboard.read(mime, writer)
        });
    }

    match result {
        // Whether the pty got the text or a paste event depends on the
        // terminal's modes; either way it went through write_pty above, in
        // chunks as the text was read.
        Ok(true) => println!("  written"),
        Ok(false) => println!("  nothing to paste"),
        Err(e) => eprintln!("paste failed: {e}"),
    }
}
// ANCHOR_END: terminal-paste

// A real embedder would show a dialog here.
fn confirm_with_user() -> bool {
    println!("  paste could inject commands; user confirmed");
    true
}

// ANCHOR: paste-safety
fn safety_example() {
    if paste::is_safe(b"hello world") {
        println!("Safe to paste");
    }

    if paste::is_safe(b"rm -rf /\n") {
        println!("Unsafe! Contains newline");
    }
}
// ANCHOR_END: paste-safety

// ANCHOR: paste-encode
fn encode_example() {
    let mut buf = [0u8; 64];
    // The input buffer is modified in place (unsafe bytes are stripped).
    let mut data = *b"hello\nworld";

    if let Ok(len) = paste::encode(&mut data, true, &mut buf) {
        println!("Encoded {len} bytes: {}", buf[0..len].escape_ascii());
    }
}
// ANCHOR_END: paste-encode

fn main() -> Result<(), Error> {
    // The base64 password of the last paste event, captured from the OK
    // packet so the example can play the program's side of the protocol.
    let event_pw = RefCell::new(Vec::new());
    let mut terminal = Terminal::new(80, 24)?;

    // Pasted bytes and paste events go to on_pty_write. Serving clipboard
    // reads is what lets the terminal send paste events at all: without
    // this callback the program could never read the clipboard, so pastes
    // stay text even when mode 5522 is enabled.
    terminal.on_pty_write(|_, data| {
        // Everything the terminal writes to the running program: the pasted
        // text, or the paste event packets when mode 5522 is enabled.
        println!("  -> pty ({} bytes): {}", data.len(), data.escape_ascii());

        // A paste event's OK packet: OSC 5522 ; type=read:status=OK:pw=<b64> ST
        let prefix = b"\x1b]5522;type=read:status=OK:pw=";

        if let Some(pw) = data.strip_prefix(prefix) {
            event_pw.replace(pw.iter().copied().take_while(|&v| v != b'\x1b').collect());
        }
    })?;

    // Serves clipboard reads. After a paste event the program's read arrives
    // with `granted` set, because it carries the event's one-time password,
    // so the embedder skips its permission prompt.
    terminal.on_clipboard_read(|_, read| {
        print!(
            r#"  clipboard read: name="{}" "#,
            read.name().escape_ascii(),
        );

        if read.granted() {
            println!("granted=yes (no prompt needed)");
        } else {
            println!("granted=no");
        }

        read.reply(
            Ok(&[effect::ClipboardReplyContent::new(
                b"text/plain",
                b"hello from the clipboard",
            )]),
            &[],
            false,
        );
    })?;

    println!("Plain paste:");
    paste_clipboard(&mut terminal, "hello world");

    println!("Paste with a newline (refused, then confirmed):");
    paste_clipboard(&mut terminal, "echo hi\n");

    // The program enables bracketed paste: newlines are safe inside the
    // frame and are preserved.
    println!("Bracketed paste (mode 2004):");
    terminal.vt_write(b"\x1b[?2004h");
    paste_clipboard(&mut terminal, "line one\nline two");

    // The program enables paste events: the clipboard's MIME types are
    // listed with a one-time password instead of writing the data.
    println!("Paste event (mode 5522):");
    terminal.vt_write(b"\x1b[?5522h");
    paste_clipboard(&mut terminal, "hello world");

    // Play the program's side: read the clipboard with the password from
    // the event. The read arrives granted and the data is served through
    // write_pty as base64 without any permission prompt.
    if !event_pw.borrow().is_empty() {
        println!("Program reads with the event password:");
        let mut read_seq = Vec::new();
        write!(
            &mut read_seq,
            "\x1b]5522;type=read:pw={}:name=UGFzdGUgZXZlbnQ=;dGV4dC9wbGFpbg==\x1b\\",
            event_pw.borrow().escape_ascii()
        )
        .unwrap();
        terminal.vt_write(&read_seq);
    }

    // Text inserted by other means (IME, drag and drop) is never an event.
    println!("IME text with mode 5522 enabled:");

    let clipboard = Clipboard { text: "committed" };
    let pasted = terminal.paste(
        paste::Options::new()
            .with_location(effect::ClipboardLocation::Standard)
            .with_source(paste::Source::Text)
            .with_allow_unsafe(true),
        &[effect::ClipboardMime::new(b"text/plain")],
        |data, writer| clipboard.read(data, writer),
    );
    if let Ok(true) = pasted {
        println!("  written");
    }

    println!("Terminal-free building blocks");
    safety_example();
    encode_example();

    Ok(())
}
