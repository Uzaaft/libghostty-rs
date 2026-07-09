//! Terminal cell style attributes.
//!
//! A style describes the visual attributes of a terminal cell, including
//! foreground, background, and underline colors, as well as flags for bold,
//! italic, underline, and other text decorations.
use std::{ffi::CStr, mem::MaybeUninit, slice};

use crate::{
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
pub struct PaletteIndex(pub ffi::ColorPaletteIndex);

impl PaletteIndex {
    #![expect(missing_docs, reason = "self-explanatory")]
    pub const BLACK: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BLACK);
    pub const RED: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_RED);
    pub const GREEN: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_GREEN);
    pub const YELLOW: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_YELLOW);
    pub const BLUE: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BLUE);
    pub const MAGENTA: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_MAGENTA);
    pub const CYAN: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_CYAN);
    pub const WHITE: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_WHITE);
    pub const BRIGHT_BLACK: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_BLACK);
    pub const BRIGHT_RED: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_RED);
    pub const BRIGHT_GREEN: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_GREEN);
    pub const BRIGHT_YELLOW: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_YELLOW);
    pub const BRIGHT_BLUE: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_BLUE);
    pub const BRIGHT_MAGENTA: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_MAGENTA);
    pub const BRIGHT_CYAN: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_CYAN);
    pub const BRIGHT_WHITE: PaletteIndex = PaletteIndex(ffi::COLOR_NAMED_BRIGHT_WHITE);
}

/// Underline style types.
#[repr(u32)]
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
            bold: value.bold,
            italic: value.italic,
            faint: value.faint,
            blink: value.blink,
            inverse: value.inverse,
            invisible: value.invisible,
            strikethrough: value.strikethrough,
            overline: value.overline,
            #[expect(clippy::cast_sign_loss, reason = "bindgen ain't perfect")]
            underline: Underline::try_from(value.underline as u32)
                .map_err(|_| Error::InvalidValue)?,
        })
    }
}

impl From<Style> for ffi::Style {
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
            #[expect(clippy::cast_possible_wrap, reason = "bindgen ain't perfect")]
            underline: u32::from(value.underline) as i32,
        }
    }
}

impl TryFrom<ffi::StyleColor> for StyleColor {
    type Error = Error;
    fn try_from(value: ffi::StyleColor) -> Result<Self> {
        Ok(match value.tag {
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
    fn from(value: StyleColor) -> Self {
        match value {
            StyleColor::None => Self {
                tag: ffi::StyleColorTag::NONE,
                value: ffi::StyleColorValue::default(),
            },
            StyleColor::Palette(PaletteIndex(palette)) => Self {
                tag: ffi::StyleColorTag::PALETTE,
                value: ffi::StyleColorValue { palette },
            },
            StyleColor::Rgb(rgb) => Self {
                tag: ffi::StyleColorTag::RGB,
                value: ffi::StyleColorValue { rgb: rgb.into() },
            },
        }
    }
}

impl From<ffi::ColorRgb> for RgbColor {
    fn from(value: ffi::ColorRgb) -> Self {
        let ffi::ColorRgb { r, g, b } = value;
        Self { r, g, b }
    }
}

impl From<RgbColor> for ffi::ColorRgb {
    fn from(value: RgbColor) -> Self {
        let RgbColor { r, g, b } = value;
        Self { r, g, b }
    }
}

/// A 256-color palette.
pub type Palette = [RgbColor; 256];

/// A 256-bit mask of palette indices.
///
/// Index `i` is set iff `(bits[i >> 6] >> (i & 63)) & 1` is 1. The mask is
/// typically initialized to zero and then populated with [`PaletteMask::set`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PaletteMask {
    bits: [u64; 4],
}

impl PaletteMask {
    /// Create an empty palette mask.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a palette index in this mask.
    pub fn set(&mut self, index: PaletteIndex) {
        let index = usize::from(index.0);
        self.bits[index / 64] |= 1_u64 << (index % 64);
    }

    /// Return whether a palette index is set in this mask.
    #[must_use]
    pub fn is_set(self, index: PaletteIndex) -> bool {
        let index = usize::from(index.0);
        (self.bits[index / 64] & (1_u64 << (index % 64))) != 0
    }
}

/// Get Ghostty's built-in default 256-color palette.
///
/// Returns Ghostty's base16 defaults, the xterm 6x6x6 color cube, and the
/// grayscale ramp.
#[must_use]
pub fn default_palette() -> Palette {
    let mut raw = [ffi::ColorRgb::default(); 256];
    unsafe { ffi::ghostty_color_palette_default(raw.as_mut_ptr()) };
    raw.map(RgbColor::from)
}

/// Generate a 256-color palette from base colors.
///
/// The base palette supplies indices 0-15, which are always preserved. If
/// `base` is [`None`], Ghostty's default palette is used. If `skip` is
/// [`None`], no extra indices are skipped. Set bits in `skip` preserve those
/// indices from `base`. The 216-color cube at indices 16-231 is generated with
/// trilinear CIELAB interpolation, and the grayscale ramp at indices 232-255 is
/// interpolated from the background to the foreground.
///
/// For light themes, `harmonious` controls whether the generated palette keeps
/// the background-to-foreground orientation. When false, Ghostty swaps the
/// light background and dark foreground so the cube and ramp run dark-to-light.
#[must_use]
pub fn generate_palette(
    base: Option<&Palette>,
    skip: Option<&PaletteMask>,
    background: RgbColor,
    foreground: RgbColor,
    harmonious: bool,
) -> Palette {
    let raw_base = base.map(|palette| palette.map(ffi::ColorRgb::from));
    let base_ptr = raw_base
        .as_ref()
        .map_or(std::ptr::null(), |palette| palette.as_ptr());
    let raw_skip = skip.map(|mask| ffi::ColorPaletteMask { bits: mask.bits });
    let skip_ptr = raw_skip.as_ref().map_or(std::ptr::null(), |mask| mask);
    let mut raw_out = [ffi::ColorRgb::default(); 256];

    unsafe {
        ffi::ghostty_color_palette_generate(
            base_ptr,
            skip_ptr,
            background.into(),
            foreground.into(),
            harmonious,
            raw_out.as_mut_ptr(),
        );
    }

    raw_out.map(RgbColor::from)
}

/// Parse an X11 color name.
///
/// The color name is resolved from Ghostty's embedded rgb.txt table. Leading
/// and trailing spaces and tabs are trimmed, and matching is ASCII
/// case-insensitive. Hex values are not accepted by this function.
pub fn parse_x11_color(name: &str) -> Result<RgbColor> {
    let mut out = MaybeUninit::uninit();
    let result =
        unsafe { ffi::ghostty_color_parse_x11(name.as_ptr().cast(), name.len(), out.as_mut_ptr()) };
    crate::error::from_result(result)?;
    Ok(unsafe { out.assume_init() }.into())
}

/// Parse a flexible Ghostty color value.
///
/// Accepts Ghostty's terminal color syntax: X11 color names, hex colors in 3-,
/// 6-, 9-, or 12-digit form (the leading `#` is optional for 3- and 6-digit
/// values), and `rgb:<red>/<green>/<blue>` or
/// `rgbi:<red>/<green>/<blue>` specifications. Leading and trailing spaces and
/// tabs are trimmed.
pub fn parse_color(value: &str) -> Result<RgbColor> {
    let mut out = MaybeUninit::uninit();
    let result =
        unsafe { ffi::ghostty_color_parse(value.as_ptr().cast(), value.len(), out.as_mut_ptr()) };
    crate::error::from_result(result)?;
    Ok(unsafe { out.assume_init() }.into())
}

/// Parse a Ghostty palette entry.
///
/// Accepts Ghostty palette config syntax: `N=COLOR`. `N` is a palette index
/// from 0 to 255 in decimal or in `0x`, `0o`, or `0b`-prefixed form. Spaces and
/// tabs around `N` and `COLOR` are ignored. `COLOR` accepts the same syntax as
/// [`parse_color`].
pub fn parse_palette_entry(value: &str) -> Result<(PaletteIndex, RgbColor)> {
    let mut index = MaybeUninit::uninit();
    let mut rgb = MaybeUninit::uninit();
    let result = unsafe {
        ffi::ghostty_color_parse_palette_entry(
            value.as_ptr().cast(),
            value.len(),
            index.as_mut_ptr(),
            rgb.as_mut_ptr(),
        )
    };
    crate::error::from_result(result)?;
    Ok((
        PaletteIndex(unsafe { index.assume_init() }),
        unsafe { rgb.assume_init() }.into(),
    ))
}

/// Calculate W3C relative luminance for an RGB color.
///
/// Returns a normalized value from 0.0 for black to 1.0 for white. See
/// <https://www.w3.org/TR/WCAG20/#relativeluminancedef>.
#[must_use]
pub fn luminance(color: RgbColor) -> f64 {
    unsafe { ffi::ghostty_color_luminance(color.into()) }
}

/// Calculate perceived luminance for an RGB color.
///
/// Returns a normalized value from 0.0 for black to 1.0 for white. Ghostty
/// treats a background color as light when this exceeds 0.5. This is not the
/// metric used internally by [`generate_palette`], which uses CIELAB lightness.
#[must_use]
pub fn perceived_luminance(color: RgbColor) -> f64 {
    unsafe { ffi::ghostty_color_perceived_luminance(color.into()) }
}

/// Calculate the WCAG contrast ratio between two RGB colors.
///
/// The contrast ratio is symmetric and ranges from 1.0 for identical colors to
/// 21.0 for black and white.
#[must_use]
pub fn contrast(a: RgbColor, b: RgbColor) -> f64 {
    unsafe { ffi::ghostty_color_contrast(a.into(), b.into()) }
}

/// An entry in Ghostty's X11 color name table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X11ColorName<'a> {
    /// Null-terminated color name.
    pub name: &'a CStr,
    /// The RGB value of the color.
    pub color: RgbColor,
}

/// Ghostty's X11 color name table.
///
/// This is a borrowed view of the static table embedded in Ghostty.
#[derive(Clone, Copy, Debug)]
pub struct X11ColorNames {
    entries: &'static [ffi::ColorX11Entry],
}

/// Iterator over Ghostty's X11 color name table.
#[derive(Clone, Debug)]
pub struct X11ColorNamesIter {
    entries: slice::Iter<'static, ffi::ColorX11Entry>,
}

impl X11ColorNames {
    /// Get the number of X11 color name entries.
    ///
    /// The returned count excludes the NULL terminator.
    #[must_use]
    pub fn len(self) -> usize {
        self.entries.len()
    }

    /// Return whether the X11 color name table is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.entries.is_empty()
    }

    /// Get an X11 color name entry by index.
    #[must_use]
    pub fn get(self, index: usize) -> Option<X11ColorName<'static>> {
        self.entries.get(index).map(x11_color_name_from_entry)
    }

    /// Iterate over the X11 color name table.
    pub fn iter(self) -> X11ColorNamesIter {
        X11ColorNamesIter {
            entries: self.entries.iter(),
        }
    }
}

impl IntoIterator for X11ColorNames {
    type Item = X11ColorName<'static>;
    type IntoIter = X11ColorNamesIter;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for X11ColorNamesIter {
    type Item = X11ColorName<'static>;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next().map(x11_color_name_from_entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl ExactSizeIterator for X11ColorNamesIter {}

fn x11_color_name_from_entry(entry: &ffi::ColorX11Entry) -> X11ColorName<'static> {
    let name = unsafe { CStr::from_ptr(entry.name) };
    X11ColorName {
        name,
        color: entry.color.into(),
    }
}

/// Get Ghostty's X11 color name table.
///
/// Entries are in rgb.txt order. Aliases are separate entries, such as
/// `"medium spring green"` and `"MediumSpringGreen"`. Names are the exact
/// supported spellings from rgb.txt; [`parse_x11_color`] also matches them
/// case-insensitively.
pub fn x11_color_names() -> X11ColorNames {
    let ptr = unsafe { ffi::ghostty_color_x11_names() };
    let len = unsafe { ffi::ghostty_color_x11_name_count() };
    let entries = unsafe { slice::from_raw_parts(ptr, len) };
    X11ColorNames { entries }
}
