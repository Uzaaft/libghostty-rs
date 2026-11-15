use libghostty_vt::{error::Error, sgr};

// ANCHOR: sgr-basic
fn basic_example() -> Result<(), Error> {
    // Create parser
    let mut parser = sgr::Parser::new()?;

    // Parse "bold, red foreground" sequence: ESC[1;31m
    parser.set_params(&[1, 31], None)?;

    // Iterate through attributes
    while let Some(attr) = parser.next()? {
        match attr {
            sgr::Attribute::Bold => println!("Bold enabled"),
            sgr::Attribute::Fg8(color) => println!("Foreground color: {color:?}"),
            _ => {}
        }
    }
    Ok(())
}
// ANCHOR_END: sgr-basic

fn advanced_example() -> Result<(), Error> {
    let mut parser = sgr::Parser::new()?;

    // Parse a complex SGR sequence from Kakoune
    // This corresponds to the escape sequence:
    // ESC[4:3;38;2;51;51;51;48;2;170;170;170;58;2;255;97;136m
    //
    // Breaking down the sequence:
    // - 4:3 = curly underline (colon-separated sub-parameters)
    // - 38;2;51;51;51 = foreground RGB color (51, 51, 51) - dark gray
    // - 48;2;170;170;170 = background RGB color (170, 170, 170) - light gray
    // - 58;2;255;97;136 = underline RGB color (255, 97, 136) - pink
    let params = [
        4u16, 3, 38, 2, 51, 51, 51, 48, 2, 170, 170, 170, 58, 2, 255, 97, 136,
    ];

    // Separator array: ':' at position 0 (between 4 and 3), ';' elsewhere
    let mut separators = params.map(|_| b';');
    separators[0] = b':';

    parser.set_params(&params, Some(&separators))?;

    println!("\nParsing Kakoune SGR sequence:");
    println!("ESC[4:3;38;2;51;51;51;48;2;170;170;170;58;2;255;97;136m\n");

    let mut count = 0usize;
    while let Some(attr) = parser.next()? {
        count += 1;
        println!("Attribute {count}: ");

        use sgr::Attribute::*;
        match attr {
            Underline(v) => println!("Underline style = {v:?}"),
            DirectColorFg(v) => println!("Foreground RGB = ({}, {}, {})", v.r, v.g, v.b),
            DirectColorBg(v) => println!("Background RGB = ({}, {}, {})", v.r, v.g, v.b),
            UnderlineColor(v) => println!("Underline color RGB = ({}, {}, {})", v.r, v.g, v.b),
            Fg8(v) => println!("Foreground 8-color = {v:?}"),
            Bg8(v) => println!("Background 8-color = {v:?}"),
            Fg256(v) => println!("Foreground 256-color = {v:?}"),
            Bg256(v) => println!("Background 256-color = {v:?}"),
            Bold => println!("Bold"),
            Italic => println!("Italic"),
            Unset => println!("Reset all attributes"),
            Unknown(_) => println!("Unknown attribute"),
            other => println!("Other attribute (tag={other:?})"),
        }
    }

    println!("\nTotal attributes parsed: {count}");
    Ok(())
}

fn main() -> Result<(), Error> {
    basic_example()?;
    advanced_example()?;
    Ok(())
}
