use crate::{Error, ffi};

pub struct Style {
    pub fg_color: StyleColor,
    pub bg_color: StyleColor,
    pub underline_color: StyleColor,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

pub enum StyleColor {
    None,
    Palette(PaletteIndex),
    Rgb(RgbColor),
}

/// RGB color value.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct RgbColor {
    /// Red color component (0-255)
    pub r: u8,
    /// Green color component (0-255)
    pub g: u8,
    /// Blue color component (0-255)
    pub b: u8,
}

/// Palette color index (0-255).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaletteIndex(pub u8);

impl PaletteIndex {
    pub const BLACK: PaletteIndex = PaletteIndex(0);
    pub const RED: PaletteIndex = PaletteIndex(1);
    pub const GREEN: PaletteIndex = PaletteIndex(2);
    pub const YELLOW: PaletteIndex = PaletteIndex(3);
    pub const BLUE: PaletteIndex = PaletteIndex(4);
    pub const MAGENTA: PaletteIndex = PaletteIndex(5);
    pub const CYAN: PaletteIndex = PaletteIndex(6);
    pub const WHITE: PaletteIndex = PaletteIndex(7);
    pub const BRIGHT_BLACK: PaletteIndex = PaletteIndex(8);
    pub const BRIGHT_RED: PaletteIndex = PaletteIndex(9);
    pub const BRIGHT_GREEN: PaletteIndex = PaletteIndex(10);
    pub const BRIGHT_YELLOW: PaletteIndex = PaletteIndex(11);
    pub const BRIGHT_BLUE: PaletteIndex = PaletteIndex(12);
    pub const BRIGHT_MAGENTA: PaletteIndex = PaletteIndex(13);
    pub const BRIGHT_CYAN: PaletteIndex = PaletteIndex(14);
    pub const BRIGHT_WHITE: PaletteIndex = PaletteIndex(15);
}

/// Underline style types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Underline {
    None = 0,
    Single = 1,
    Double = 2,
    Curly = 3,
    Dotted = 4,
    Dashed = 5,
}

//----------------------------------
// Conversion to and from FFI types
//----------------------------------

impl Style {
    pub(crate) fn from_raw(value: ffi::GhosttyStyle) -> Result<Self, Error> {
        Ok(Self {
            fg_color: StyleColor::from_raw(value.fg_color)?,
            bg_color: StyleColor::from_raw(value.bg_color)?,
            underline_color: StyleColor::from_raw(value.underline_color)?,
            bold: value.bold,
            italic: value.italic,
            faint: value.faint,
            blink: value.blink,
            inverse: value.inverse,
            invisible: value.invisible,
            strikethrough: value.strikethrough,
            overline: value.overline,
            underline: Underline::from_raw(value.underline as u32)?,
        })
    }
}

impl StyleColor {
    pub(crate) fn from_raw(value: ffi::GhosttyStyleColor) -> Result<Self, Error> {
        Ok(match value.tag {
            ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE => Self::None,
            ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_PALETTE => {
                Self::Palette(PaletteIndex(unsafe { value.value.palette }))
            }
            ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_RGB => {
                Self::Rgb(unsafe { value.value.rgb }.into())
            }
            _ => return Err(Error::InvalidValue),
        })
    }
}

impl From<ffi::GhosttyColorRgb> for RgbColor {
    fn from(value: ffi::GhosttyColorRgb) -> Self {
        let ffi::GhosttyColorRgb { r, g, b } = value;
        Self { r, g, b }
    }
}

impl Underline {
    /// This should never return None, but just to be safe.
    pub(crate) fn from_raw(value: ffi::GhosttySgrUnderline) -> Result<Self, Error> {
        Ok(match value {
            0 => Self::None,
            1 => Self::Single,
            2 => Self::Double,
            3 => Self::Curly,
            4 => Self::Dotted,
            5 => Self::Dashed,
            _ => return Err(Error::InvalidValue),
        })
    }
}
