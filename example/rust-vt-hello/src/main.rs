//! The smallest complete program: create a terminal, write text with a color
//! sequence in it, and print the screen. This is the program on the main page.
// ANCHOR: hello
use libghostty_vt::{error::Error, fmt::Formatter, Terminal};

fn main() -> Result<(), Error> {
    // Create an 80x24 terminal with the default allocator.
    let mut terminal = Terminal::new(80, 24)?;

    // Write the bytes a program would send. They can come from anywhere: a pty,
    // a network connection, a recording. The escape sequences make "world" bold
    // and green.
    terminal.vt_write(b"Hello, \x1b[1;32mworld\x1b[0m!\r\n");

    // Read the screen back. A new formatter emits plain text, so the styles
    // don't appear in the output, but you can set options to enable that.
    let mut formatter = Formatter::new(&terminal)?;
    let buf = formatter.format_alloc(None)?;

    println!("{}", buf.escape_ascii());
    Ok(())
}
// ANCHOR_END: hello
