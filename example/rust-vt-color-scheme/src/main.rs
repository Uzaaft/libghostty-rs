//! Encode a color scheme report, the reply to a CSI ? 996 n query.

use libghostty_vt::report::ColorScheme;

// ANCHOR: color-scheme-report-encode
fn main() {
    let mut buf = [0u8; 16];
    if let Ok(len) = ColorScheme::Dark.encode(&mut buf) {
        println!("Encoded {len} bytes: {}", buf[0..len].escape_ascii());
    }
}
// ANCHOR_END: color-scheme-report-encode
