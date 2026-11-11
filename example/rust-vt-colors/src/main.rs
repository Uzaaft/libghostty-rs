//! Set the default colors and palette, then read effective and default
//! values as an OSC 10 override layers on top.

use libghostty_vt::{
    color::{Palette, PaletteIndex, Rgb},
    error::Error,
    Terminal,
};

#[rustfmt::skip]
// ANCHOR: colors-set-defaults
/// Set up a dark color theme with custom palette entries.
fn set_color_theme(terminal: &mut Terminal<'_, '_>) -> Result<(), Error> {
    // Set default foreground (light gray) and background (dark)
    terminal
        .set_default_fg_color(Some(Rgb { r: 0xDD, g: 0xDD, b: 0xDD }))?
        .set_default_bg_color(Some(Rgb { r: 0x1E, g: 0x1E, b: 0x2E }))?
        .set_default_cursor_color(Some(Rgb { r: 0xF5, g: 0xE0, b: 0xDC }))?;

    // Set a custom palette — start from the built-in default and override
    // the first 8 entries with a custom dark theme.
    let mut palette = terminal.color_palette()?;
    palette.set(PaletteIndex::BLACK,   Rgb { r: 0x45, g: 0x47, b: 0x5A });
    palette.set(PaletteIndex::RED,     Rgb { r: 0xF3, g: 0x8B, b: 0xA8 });
    palette.set(PaletteIndex::GREEN,   Rgb { r: 0xA6, g: 0xE3, b: 0xA1 });
    palette.set(PaletteIndex::YELLOW,  Rgb { r: 0xF9, g: 0xE2, b: 0xAF });
    palette.set(PaletteIndex::BLUE,    Rgb { r: 0x89, g: 0xB4, b: 0xFA });
    palette.set(PaletteIndex::MAGENTA, Rgb { r: 0xF5, g: 0xC2, b: 0xE7 });
    palette.set(PaletteIndex::CYAN,    Rgb { r: 0x94, g: 0xE2, b: 0xD5 });
    palette.set(PaletteIndex::WHITE,   Rgb { r: 0xBA, g: 0xC2, b: 0xDE });
    terminal.set_default_color_palette(Some(palette))?;

    Ok(())
}
// ANCHOR_END: colors-set-defaults

// ANCHOR: colors-read
/// Print the effective and default values for a color, showing how
/// OSC overrides layer on top of defaults.
fn print_color(name: &str, effective: Option<Rgb>, default: Option<Rgb>) {
    match effective {
        Some(v) => print!("  {name:>12} effective: #{:02X}{:02X}{:02X}", v.r, v.g, v.b),
        None => print!("  {name:>12} effective: (not set)"),
    }
    match default {
        Some(v) => println!("  default: #{:02X}{:02X}{:02X}", v.r, v.g, v.b),
        None => println!("  default: (not set)"),
    }
}

fn print_all_colors(terminal: &Terminal<'_, '_>, label: &str) -> Result<(), Error> {
    println!("{label}:");
    print_color(
        "foreground",
        terminal.fg_color()?,
        terminal.default_fg_color()?,
    );
    print_color(
        "background",
        terminal.bg_color()?,
        terminal.default_bg_color()?,
    );
    print_color(
        "cursor",
        terminal.cursor_color()?,
        terminal.default_cursor_color()?,
    );

    // Show palette index 0 (black) as an example
    let palette = terminal.color_palette()?;
    let first = palette.get(PaletteIndex(0));
    print!(
        "  {:>12} effective: #{:02X}{:02X}{:02X}",
        "palette[0]", first.r, first.g, first.b
    );

    let palette = terminal.default_color_palette()?;
    let first = palette.get(PaletteIndex(0));
    println!("  default: #{:02X}{:02X}{:02X}", first.r, first.g, first.b);

    Ok(())
}
// ANCHOR_END: colors-read

// ANCHOR: colors-main
fn main() -> Result<(), Error> {
    // Create a terminal
    let mut terminal = Terminal::new(80, 24)?;

    // Before setting any colors, everything is unset
    print_all_colors(&terminal, "Before setting defaults");

    // Set our color theme defaults
    set_color_theme(&mut terminal);
    print_all_colors(&terminal, "\nAfter setting defaults");

    // Simulate an OSC override (e.g. a program running inside the
    // terminal changes the foreground via OSC 10)
    terminal.vt_write(b"\x1B]10;rgb:FF/00/00\x1B\\");
    print_all_colors(&terminal, "\nAfter OSC foreground override");

    // Clear the foreground default — the OSC override is still active
    terminal.set_default_fg_color(None);
    print_all_colors(&terminal, "\nAfter clearing foreground default");

    Ok(())
}
// ANCHOR_END: colors-main
