use libghostty_vt::{error::Error, osc};

fn main() -> Result<(), Error> {
    let mut parser = osc::Parser::new()?;

    // Setup change window title command to change the title to "hello"
    parser.next_byte(b'0');
    parser.next_byte(b';');
    for &b in b"hello" {
        parser.next_byte(b);
    }

    // End parsing and get command
    let command = parser.end(b'\x07');

    // Get and print command type along with title
    let ty = command.command_type();
    println!("Command type: {ty:?}");
    if let osc::CommandType::ChangeWindowTitle { title } = ty {
        println!("Title: {}", title.escape_ascii());
    }
    Ok(())
}
