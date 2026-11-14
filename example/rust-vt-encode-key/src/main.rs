//! Encode Ctrl+C as an escape sequence with the Kitty keyboard protocol.

// ANCHOR: key-encode
use libghostty_vt::{error::Error, key};

fn main() -> Result<(), Error> {
    // Create encoder
    let mut encoder = key::Encoder::new()?;

    // Enable Kitty keyboard protocol with all features
    encoder.set_kitty_flags(key::KittyKeyFlags::ALL);

    // Create and configure key event for Ctrl+C press
    let mut event = key::Event::new()?;
    event
        .set_action(key::Action::Press)
        .set_key(key::Key::C)
        .set_mods(key::Mods::CTRL)
        .set_unshifted_codepoint('c');

    // Encode the key event
    let mut buf = [0u8; 128];
    let len = encoder.encode(&event, &mut buf)?;

    // Use the encoded sequence (e.g., write to terminal)
    println!("{}", buf[0..len].escape_ascii());
    Ok(())
}
// ANCHOR_END: key-encode
