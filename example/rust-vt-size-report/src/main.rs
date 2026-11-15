use libghostty_vt::{
    error::Error,
    report::{SizeReport, SizeReportSize},
};

// ANCHOR: size-report-encode
fn main() -> Result<(), Error> {
    let size = SizeReportSize {
        rows: 24,
        columns: 80,
        cell_width: 9,
        cell_height: 18,
    };

    let mut buf = [0u8; 64];
    let written = SizeReport::Mode2048.encode(&size, &mut buf)?;

    println!(
        "Encoded {written} bytes: {}",
        buf[0..written].escape_ascii()
    );
    Ok(())
}
// ANCHOR_END: size-report-encode
