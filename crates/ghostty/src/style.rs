//! Terminal cell style attributes.
//!
//! A style describes the visual attributes of a terminal cell, including
//! foreground, background, and underline colors, as well as flags for bold,
//! italic, underline, and other text decorations.
use std::mem::MaybeUninit;

use crate::{
    error::{Error, Result},
    ffi,
};

/// Style identifier type.
///
/// Used to look up the full style from a grid reference.
/// Obtain this from a cell via [`Cell::style_id`][crate::screen::Cell::style_id].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id(pub(crate) ffi::GhosttyStyleId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl Style {
    pub fn is_default(self) -> bool {
        let raw = ffi::GhosttyStyle::from(self);
        unsafe { ffi::ghostty_style_is_default(&raw) }
    }
}

impl Default for Style {
    fn default() -> Self {
        let mut style = MaybeUninit::zeroed();
        unsafe {
            ffi::ghostty_style_default(style.as_mut_ptr());
        }

        // SAFETY: We trust the function above to initialize everything correctly
        Self::try_from(unsafe { style.assume_init() })
            .expect("ghostty_style_default to init valid Style")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, int_enum::IntEnum)]
#[non_exhaustive]
pub enum Underline {
    None = ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_NONE,
    Single = ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_SINGLE,
    Double = ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DOUBLE,
    Curly = ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_CURLY,
    Dotted = ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DOTTED,
    Dashed = ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DASHED,
}

//----------------------------------
// Conversion to and from FFI types
//----------------------------------

impl TryFrom<ffi::GhosttyStyle> for Style {
    type Error = Error;
    fn try_from(value: ffi::GhosttyStyle) -> Result<Self> {
        Ok(Self {
            fg_color: StyleColor::try_from(value.fg_color)?,
            bg_color: StyleColor::try_from(value.bg_color)?,
            underline_color: StyleColor::try_from(value.underline_color)?,
            bold: value.bold,
            italic: value.italic,
            faint: value.faint,
            blink: value.blink,
            inverse: value.inverse,
            invisible: value.invisible,
            strikethrough: value.strikethrough,
            overline: value.overline,
            underline: Underline::try_from(value.underline as u32)
                .map_err(|_| Error::InvalidValue)?,
        })
    }
}

impl From<Style> for ffi::GhosttyStyle {
    fn from(value: Style) -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            fg_color: value.fg_color.into(),
            bg_color: value.bg_color.into(),
            underline_color: value.underline_color.into(),
            bold: value.bold,
            italic: value.italic,
            faint: value.faint,
            blink: value.blink,
            inverse: value.inverse,
            invisible: value.invisible,
            strikethrough: value.strikethrough,
            overline: value.overline,
            underline: u32::from(value.underline) as i32,
        }
    }
}

impl TryFrom<ffi::GhosttyStyleColor> for StyleColor {
    type Error = Error;
    fn try_from(value: ffi::GhosttyStyleColor) -> Result<Self> {
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

impl From<StyleColor> for ffi::GhosttyStyleColor {
    fn from(value: StyleColor) -> Self {
        match value {
            StyleColor::None => Self {
                tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE,
                value: ffi::GhosttyStyleColorValue::default(),
            },
            StyleColor::Palette(PaletteIndex(palette)) => Self {
                tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_PALETTE,
                value: ffi::GhosttyStyleColorValue { palette },
            },
            StyleColor::Rgb(rgb) => Self {
                tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE,
                value: ffi::GhosttyStyleColorValue { rgb: rgb.into() },
            },
        }
    }
}

impl From<ffi::GhosttyColorRgb> for RgbColor {
    fn from(value: ffi::GhosttyColorRgb) -> Self {
        let ffi::GhosttyColorRgb { r, g, b } = value;
        Self { r, g, b }
    }
}

impl From<RgbColor> for ffi::GhosttyColorRgb {
    fn from(value: RgbColor) -> Self {
        let RgbColor { r, g, b } = value;
        Self { r, g, b }
    }
}
