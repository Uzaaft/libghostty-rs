//! Encode a focus gained event as CSI I, for programs that turn on
//! focus reporting (mode 1004).

// ANCHOR: focus-encode
use libghostty_vt::{error::Error, focus};

fn main() -> Result<(), Error> {
    let mut buf = [0u8; 8];
    if let Ok(len) = focus::Event::Gained.encode(&mut buf) {
        println!("Encoded {} bytes: {}", len, buf[0..len].escape_ascii());
    }

    Ok(())
}
// ANCHOR_END: focus-encode
