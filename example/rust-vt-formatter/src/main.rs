use libghostty_vt::{
    error::Error,
    fmt::{Format, Formatter},
    Terminal,
};
use std::io;

struct Counter<W> {
    writer: W,
    count: usize,
}

impl<W: io::Write> io::Write for Counter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.writer.write(buffer)?;
        self.count += written;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

fn main() -> Result<(), Error> {
    // Create a terminal with a small grid
    let mut terminal = Terminal::new(80, 24)?;

    // Write VT-encoded content into the terminal to exercise various
    // cursor movement and styling sequences.
    let commands: [&'static [u8]; _] = [
        b"Line 1: Hello World!\r\n",        // Simple text on row 1
        b"Line 2: \x1b[1mBold\x1b[0m and ", // Bold text on row 2
        b"\x1b[4mUnderline\x1b[0m\r\n",     //
        b"Line 3: placeholder\r\n",         // Will be overwritten below
        b"\x1b[3;1H",                       // CUP: move cursor back to row 3, col 1
        b"\x1b[2K",                         // EL:  erase the entire line
        b"Line 3: Overwritten!\r\n",        // Rewrite row 3 with new content
        b"\x1b[5;10H",                      // CUP: jump to row 5, col 10
        b"Placed at (5,10)",                // Write at that position
        b"\x1b[1;72H",                      // CUP: jump to row 1, col 72
        b"RIGHT->",                         // Near the right edge of row 1
    ];
    for cmd in commands {
        terminal.vt_write(cmd);
    }

    // ANCHOR: formatter-new
    // Create a formatter for the terminal and ask for plain text with
    // trailing whitespace trimmed. A new formatter already does both, but
    // setting them shows how options work and makes the output we want
    // explicit.
    let mut formatter = Formatter::new(&terminal)?;
    formatter.set_format(Format::Plain)?.set_trim(true)?;
    // ANCHOR_END: formatter-new

    // Stream the formatted output directly to stdout.
    let mut stdout = Counter {
        writer: io::stdout(),
        count: 0,
    };
    println!("Formatted output:");
    unsafe {
        // SAFETY: The Write implementation does not involve the formatter
        // or the terminal at all.
        formatter.format_write(&mut stdout)?;
    }
    println!("\n({} bytes)", stdout.count);

    Ok(())
}
