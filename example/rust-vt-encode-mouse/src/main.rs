//! Encode a left button press at a pixel position as an SGR mouse sequence.

// ANCHOR: mouse-encode
use libghostty_vt::{error::Error, mouse};

fn main() -> Result<(), Error> {
    // Create encoder
    let mut encoder = mouse::Encoder::new()?;

    // Configure SGR format with normal tracking
    encoder
        .set_tracking_mode(mouse::TrackingMode::Normal)
        .set_format(mouse::Format::Sgr);

    // Set terminal geometry for coordinate mapping
    encoder.set_size(mouse::EncoderSize {
        screen_width: 800,
        screen_height: 600,
        cell_width: 10,
        cell_height: 20,
        ..mouse::EncoderSize::default()
    })?;

    // Create and configure a left button press event
    let mut event = mouse::Event::new()?;
    event
        .set_action(mouse::Action::Press)
        .set_button(Some(mouse::Button::Left))
        .set_position(mouse::Position { x: 50.0, y: 40.0 });

    // Encode the mouse event
    let mut buf = [0u8; 128];
    let len = encoder.encode(&event, &mut buf)?;

    // Use the encoded sequence (e.g., write to terminal)
    println!("{}", buf[0..len].escape_ascii());
    Ok(())
}
// ANCHOR_END: mouse-encode
