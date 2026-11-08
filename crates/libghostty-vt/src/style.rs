//! Read a cell's colors and text attributes.
//!
//! A [`Style`] is how a cell looks: its foreground, background, and
//! underline colors, and flags for bold, italic, underline, and other
//! text decorations.
//!
//! You get one from [`GridRef::style`](crate::grid_ref::GridRef::style), or
//! from a render state when you draw. Its colors can be unset, which means
//! you use your default.
// FIXME: Transclude examples
use std::mem::MaybeUninit;

use crate::{
    color::{PaletteIndex, Rgb},
    error::{Error, Result},
    ffi,
};

/// Style identifier type.
///
/// Used to look up the full style from a grid reference.
/// Obtain this from a cell via [`Cell::style_id`][crate::screen::Cell::style_id].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id(pub(crate) ffi::StyleId);

/// Terminal cell style attributes.
///
/// A style describes the visual attributes of a terminal cell, including
/// foreground, background, and underline colors, as well as flags for bold,
/// italic, underline, and other text decorations.
#[expect(
    clippy::struct_excessive_bools,
    reason = "style attributes should be just a bunch of bools"
)]
#[expect(missing_docs, reason = "self-explanatory")]
// FIXME: Replace with FFI wrapper. Don't model all the fields for API stability
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
    /// Check if a style is the default style.
    ///
    /// Returns true if all colors are unset and all flags are off.
    #[must_use]
    pub fn is_default(self) -> bool {
        let raw = ffi::Style::from(self);
        unsafe { ffi::ghostty_style_is_default(&raw const raw) }
    }
}

impl Default for Style {
    /// Get the default style.
    ///
    /// Initializes the style to the default values (no colors, no flags).
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

/// A color used in a style attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleColor {
    /// Unset.
    None,
    /// Palette index.
    Palette(PaletteIndex),
    /// Direct RGB value.
    Rgb(Rgb),
}

/// Underline style types.
#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, int_enum::IntEnum)]
#[non_exhaustive]
#[expect(missing_docs, reason = "self-explanatory")]
pub enum Underline {
    None = ffi::SgrUnderline::NONE,
    Single = ffi::SgrUnderline::SINGLE,
    Double = ffi::SgrUnderline::DOUBLE,
    Curly = ffi::SgrUnderline::CURLY,
    Dotted = ffi::SgrUnderline::DOTTED,
    Dashed = ffi::SgrUnderline::DASHED,
}

//----------------------------------
// Conversion to and from FFI types
//----------------------------------

impl TryFrom<ffi::Style> for Style {
    type Error = Error;
    fn try_from(value: ffi::Style) -> Result<Self> {
        Ok(Self {
            fg_color: StyleColor::try_from(value.fg_color)?,
            bg_color: StyleColor::try_from(value.bg_color)?,
            underline_color: StyleColor::try_from(value.underline_color)?,
            bold: value.flags & ffi::STYLE_FLAG_BOLD != 0,
            italic: value.flags & ffi::STYLE_FLAG_ITALIC != 0,
            faint: value.flags & ffi::STYLE_FLAG_FAINT != 0,
            blink: value.flags & ffi::STYLE_FLAG_BLINK != 0,
            inverse: value.flags & ffi::STYLE_FLAG_INVERSE != 0,
            invisible: value.flags & ffi::STYLE_FLAG_INVISIBLE != 0,
            strikethrough: value.flags & ffi::STYLE_FLAG_STRIKETHROUGH != 0,
            overline: value.flags & ffi::STYLE_FLAG_OVERLINE != 0,
            underline: Underline::try_from(i32::from(
                (value.flags & ffi::STYLE_UNDERLINE_MASK) >> ffi::STYLE_UNDERLINE_SHIFT,
            ))
            .map_err(|_| Error::Unsupported)?,
        })
    }
}

impl From<Style> for ffi::Style {
    fn from(value: Style) -> Self {
        Self {
            fg_color: value.fg_color.into(),
            bg_color: value.bg_color.into(),
            underline_color: value.underline_color.into(),
            flags: (u16::from(value.bold) * ffi::STYLE_FLAG_BOLD)
                | (u16::from(value.italic) * ffi::STYLE_FLAG_ITALIC)
                | (u16::from(value.faint) * ffi::STYLE_FLAG_FAINT)
                | (u16::from(value.blink) * ffi::STYLE_FLAG_BLINK)
                | (u16::from(value.inverse) * ffi::STYLE_FLAG_INVERSE)
                | (u16::from(value.invisible) * ffi::STYLE_FLAG_INVISIBLE)
                | (u16::from(value.strikethrough) * ffi::STYLE_FLAG_STRIKETHROUGH)
                | (u16::from(value.overline) * ffi::STYLE_FLAG_OVERLINE)
                | ((value.underline as u16) << ffi::STYLE_UNDERLINE_SHIFT),
            reserved: 0,
        }
    }
}

impl TryFrom<ffi::StyleColor> for StyleColor {
    type Error = Error;
    fn try_from(value: ffi::StyleColor) -> Result<Self> {
        Ok(match i32::from(value.tag) {
            ffi::StyleColorTag::NONE => Self::None,
            ffi::StyleColorTag::PALETTE => {
                Self::Palette(PaletteIndex(unsafe { value.value.palette }))
            }
            ffi::StyleColorTag::RGB => Self::Rgb(unsafe { value.value.rgb }.into()),
            _ => return Err(Error::InvalidValue),
        })
    }
}

impl From<StyleColor> for ffi::StyleColor {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the C color tags fit in their documented one-byte field"
    )]
    fn from(value: StyleColor) -> Self {
        match value {
            StyleColor::None => Self {
                tag: ffi::StyleColorTag::NONE as u8,
                value: ffi::StyleColorValue::default(),
            },
            StyleColor::Palette(PaletteIndex(palette)) => Self {
                tag: ffi::StyleColorTag::PALETTE as u8,
                // The frozen color layout requires unused bytes to be zero.
                value: ffi::StyleColorValue {
                    rgb: ffi::ColorRgb {
                        r: palette,
                        g: 0,
                        b: 0,
                    },
                },
            },
            StyleColor::Rgb(rgb) => Self {
                tag: ffi::StyleColorTag::RGB as u8,
                value: ffi::StyleColorValue { rgb: rgb.into() },
            },
        }
    }
}
