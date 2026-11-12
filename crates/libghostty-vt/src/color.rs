//! Parse colors, generate palettes, and look up X11 color names.
//!
//! These are Ghostty's own color rules, so you don't have to write them again: the color strings its config and theme files accept, the 256-color palette the terminal uses, and the X11 color names.
//!
//! Use them when your application has to agree with Ghostty about what a color means, or to make UI decisions from luminance and contrast values. You don't need a terminal.
//!
//! # Example
//! ```
//! use libghostty_vt::color::Rgb;
//! let color: Rgb = "#abc".parse().unwrap();
//! ```
// FIXME: link to transcluded examples and guide
use std::{ffi::CStr, mem::MaybeUninit, slice};

use crate::{Terminal, error::Result, ffi};

/// Methods related to [colors](crate::color).
impl Terminal<'_, '_> {
    /// The effective foreground color (override or default).
    pub fn fg_color(&self) -> Result<Option<Rgb>> {
        self.get_optional::<ffi::ColorRgb>(ffi::TerminalData::COLOR_FOREGROUND)
            .map(|v| v.map(Into::into))
    }
    /// The default foreground color (ignoring any OSC override).
    pub fn default_fg_color(&self) -> Result<Option<Rgb>> {
        self.get_optional::<ffi::ColorRgb>(ffi::TerminalData::COLOR_FOREGROUND_DEFAULT)
            .map(|v| v.map(Into::into))
    }
    /// Set the default foreground color.
    pub fn set_default_fg_color(&mut self, v: Option<Rgb>) -> Result<&mut Self> {
        self.set_optional(
            ffi::TerminalOption::COLOR_FOREGROUND,
            v.map(ffi::ColorRgb::from).as_ref(),
        )?;
        Ok(self)
    }

    /// The effective background color (override or default).
    pub fn bg_color(&self) -> Result<Option<Rgb>> {
        self.get_optional::<ffi::ColorRgb>(ffi::TerminalData::COLOR_BACKGROUND)
            .map(|v| v.map(Into::into))
    }
    /// The default background color (ignoring any OSC override).
    pub fn default_bg_color(&self) -> Result<Option<Rgb>> {
        self.get_optional::<ffi::ColorRgb>(ffi::TerminalData::COLOR_BACKGROUND_DEFAULT)
            .map(|v| v.map(Into::into))
    }
    /// Set the default background color.
    pub fn set_default_bg_color(&mut self, v: Option<Rgb>) -> Result<&mut Self> {
        self.set_optional(
            ffi::TerminalOption::COLOR_BACKGROUND,
            v.map(ffi::ColorRgb::from).as_ref(),
        )?;
        Ok(self)
    }

    /// The effective cursor color (override or default).
    pub fn cursor_color(&self) -> Result<Option<Rgb>> {
        self.get_optional::<ffi::ColorRgb>(ffi::TerminalData::COLOR_CURSOR)
            .map(|v| v.map(Into::into))
    }
    /// The default cursor color (ignoring any OSC override).
    pub fn default_cursor_color(&self) -> Result<Option<Rgb>> {
        self.get_optional::<ffi::ColorRgb>(ffi::TerminalData::COLOR_CURSOR_DEFAULT)
            .map(|v| v.map(Into::into))
    }
    /// Set the default cursor color.
    pub fn set_default_cursor_color(&mut self, v: Option<Rgb>) -> Result<&mut Self> {
        self.set_optional(
            ffi::TerminalOption::COLOR_CURSOR,
            v.map(ffi::ColorRgb::from).as_ref(),
        )?;
        Ok(self)
    }

    /// The current 256-color palette.
    pub fn color_palette(&self) -> Result<Palette> {
        self.get::<RawPalette>(Data::COLOR_PALETTE)
            .map(Palette::from)
    }
    /// The default 256-color palette (ignoring any OSC overrides).
    pub fn default_color_palette(&self) -> Result<Palette> {
        self.get::<RawPalette>(Data::COLOR_PALETTE_DEFAULT)
            .map(Palette::from)
    }
    /// Set the default 256-color palette.
    pub fn set_default_color_palette(&mut self, v: Option<Palette>) -> Result<&mut Self> {
        self.set_optional::<RawPalette>(
            Opt::COLOR_PALETTE,
            v.map(std::convert::Into::into).as_ref(),
        )?;
        Ok(self)
    }
}

/// RGB color value.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rgb {
    /// Red color component (0-255)
    pub r: u8,
    /// Green color component (0-255)
    pub g: u8,
    /// Blue color component (0-255)
    pub b: u8,
}
impl Rgb {
    /// Parse an X11 color name.
    ///
    /// The color name is resolved from Ghostty's embedded rgb.txt table. Leading
    /// and trailing spaces and tabs are trimmed, and matching is ASCII
    /// case-insensitive. Hex values are not accepted by this function.
    pub fn parse_x11_color(name: impl AsRef<[u8]>) -> Result<Self> {
        // Ditto (see `Palette::parse_palette_entry`): upstream takes bytes.
        let name = name.as_ref();
        let mut out = MaybeUninit::uninit();
        let result = unsafe {
            ffi::ghostty_color_parse_x11(name.as_ptr().cast(), name.len(), out.as_mut_ptr())
        };
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
    pub fn parse(value: impl AsRef<[u8]>) -> Result<Self> {
        // Ditto (see `Palette::parse_palette_entry`): upstream takes bytes.
        let value = value.as_ref();
        let mut out = MaybeUninit::uninit();
        let result = unsafe {
            ffi::ghostty_color_parse(value.as_ptr().cast(), value.len(), out.as_mut_ptr())
        };
        crate::error::from_result(result)?;
        Ok(unsafe { out.assume_init() }.into())
    }

    /// Calculate W3C relative luminance for an RGB color.
    ///
    /// Returns a normalized value from 0.0 for black to 1.0 for white. See
    /// <https://www.w3.org/TR/WCAG20/#relativeluminancedef>.
    #[must_use]
    pub fn luminance(self) -> f64 {
        let this: ffi::ColorRgb = self.into();
        unsafe { ffi::ghostty_color_luminance(&raw const this) }
    }

    /// Calculate perceived luminance for an RGB color.
    ///
    /// Returns a normalized value from 0.0 for black to 1.0 for white. Ghostty
    /// treats a background color as light when this exceeds 0.5. This is not the
    /// metric used internally by [`Palette::generate`], which uses CIELAB lightness.
    #[must_use]
    pub fn perceived_luminance(self) -> f64 {
        let this: ffi::ColorRgb = self.into();
        unsafe { ffi::ghostty_color_perceived_luminance(&raw const this) }
    }

    /// Calculate the WCAG contrast ratio between two RGB colors.
    ///
    /// The contrast ratio is symmetric and ranges from 1.0 for identical colors to
    /// 21.0 for black and white.
    #[must_use]
    pub fn contrast(self, other: Rgb) -> f64 {
        let this: ffi::ColorRgb = self.into();
        let that: ffi::ColorRgb = other.into();
        unsafe { ffi::ghostty_color_contrast(&raw const this, &raw const that) }
    }
}
impl std::str::FromStr for Rgb {
    type Err = crate::error::Error;

    fn from_str(v: &str) -> Result<Rgb> {
        Rgb::parse(v)
    }
}
impl From<ffi::ColorRgb> for Rgb {
    fn from(value: ffi::ColorRgb) -> Self {
        let ffi::ColorRgb { r, g, b } = value;
        Self { r, g, b }
    }
}
impl From<Rgb> for ffi::ColorRgb {
    fn from(value: Rgb) -> Self {
        let Rgb { r, g, b } = value;
        Self { r, g, b }
    }
}

/// A 256-color palette.
#[derive(Clone, Copy, Debug)]
pub struct Palette(pub [Rgb; 256]);

// Saves a bit of typing
pub(crate) type RawPalette = [ffi::ColorRgb; 256];

impl From<RawPalette> for Palette {
    fn from(v: RawPalette) -> Self {
        Self(v.map(Rgb::from))
    }
}
impl From<Palette> for RawPalette {
    fn from(v: Palette) -> Self {
        v.0.map(ffi::ColorRgb::from)
    }
}

impl Default for Palette {
    /// Get Ghostty's built-in default 256-color palette.
    ///
    /// Returns Ghostty's base16 defaults, the xterm 6x6x6 color cube, and the
    /// grayscale ramp.
    fn default() -> Self {
        let mut raw = [ffi::ColorRgb::default(); 256];
        unsafe { ffi::ghostty_color_palette_default(raw.as_mut_ptr()) };
        raw.into()
    }
}

impl Palette {
    /// Get the color at the given palette index.
    #[must_use]
    pub fn get(&self, index: PaletteIndex) -> Rgb {
        self.0[index.0 as usize]
    }

    /// Set the color at the given palette index.
    pub fn set(&mut self, index: PaletteIndex, color: Rgb) {
        self.0[index.0 as usize] = color;
    }

    /// Parse a Ghostty palette entry.
    ///
    /// Accepts Ghostty palette config syntax: `N=COLOR`. `N` is a palette index
    /// from 0 to 255 in decimal or in `0x`, `0o`, or `0b`-prefixed form. Spaces and
    /// tabs around `N` and `COLOR` are ignored. `COLOR` accepts the same syntax as
    /// [`Rgb::parse`].
    pub fn parse_palette_entry(value: impl AsRef<[u8]>) -> Result<(PaletteIndex, Rgb)> {
        // Take `AsRef<[u8]>` rather than `&str`: upstream takes a byte
        // buffer and length with no UTF-8 promise, so strings pass straight
        // in, and so do byte strings (e.g. read from a config file) without
        // a UTF-8 check first. Input that isn't valid syntax, non-ASCII
        // included, is rejected by upstream as `Error::InvalidValue`.
        let value = value.as_ref();
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
    pub fn generate(
        base: Option<&Self>,
        skip: Option<&PaletteMask>,
        background: Rgb,
        foreground: Rgb,
        harmonious: bool,
    ) -> Self {
        let raw_base = base.map(|palette| palette.0.map(ffi::ColorRgb::from));
        let base_ptr = raw_base
            .as_ref()
            .map_or(std::ptr::null(), |palette| palette.as_ptr());
        let skip_ptr = skip
            .as_ref()
            .map_or(std::ptr::null(), |mask| &raw const mask.0);
        let mut raw_out = [ffi::ColorRgb::default(); 256];
        let bg: ffi::ColorRgb = background.into();
        let fg: ffi::ColorRgb = foreground.into();

        unsafe {
            ffi::ghostty_color_palette_generate(
                base_ptr,
                skip_ptr,
                &raw const bg,
                &raw const fg,
                harmonious,
                raw_out.as_mut_ptr(),
            );
        }

        raw_out.into()
    }
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

/// A 256-bit mask of palette indices.
///
/// The mask is typically initialized to zero and then populated with
/// [`PaletteMask::set`].
#[derive(Clone, Copy, Debug, Default)]
pub struct PaletteMask(ffi::ColorPaletteMask);

impl PaletteMask {
    /// Create an empty palette mask.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a palette index in this mask.
    pub fn set(&mut self, index: PaletteIndex) {
        let index = usize::from(index.0);
        self.0.bits[index / 64] |= 1_u64 << (index % 64);
    }

    /// Clear a palette index in this mask.
    pub fn unset(&mut self, index: PaletteIndex) {
        let index = usize::from(index.0);
        self.0.bits[index / 64] &= !(1_u64 << (index % 64));
    }

    /// Return whether a palette index is set in this mask.
    #[must_use]
    pub fn is_set(self, index: PaletteIndex) -> bool {
        let index = usize::from(index.0);
        (self.0.bits[index / 64] & (1_u64 << (index % 64))) != 0
    }
}
impl PartialEq for PaletteMask {
    fn eq(&self, v: &Self) -> bool {
        self.0.bits.eq(&v.0.bits)
    }
}
impl Eq for PaletteMask {}

/// Ghostty's X11 color name table.
///
/// This is a borrowed view of the static table embedded in Ghostty.
#[derive(Clone, Copy, Debug)]
pub struct X11ColorNames {
    entries: &'static [ffi::ColorX11Entry],
}

impl Default for X11ColorNames {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over Ghostty's X11 color name table.
#[derive(Clone, Debug)]
pub struct X11ColorNamesIter {
    entries: slice::Iter<'static, ffi::ColorX11Entry>,
}

impl X11ColorNames {
    /// Get Ghostty's X11 color name table.
    ///
    /// Entries are in rgb.txt order. Aliases are separate entries, such as
    /// `"medium spring green"` and `"MediumSpringGreen"`. Names are the exact
    /// supported spellings from rgb.txt; [`Rgb::parse_x11_color`] also matches them
    /// case-insensitively.
    #[must_use]
    pub fn new() -> Self {
        let ptr = unsafe { ffi::ghostty_color_x11_names() };
        let len = unsafe { ffi::ghostty_color_x11_name_count() };
        let entries = unsafe { slice::from_raw_parts(ptr, len) };
        Self { entries }
    }

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
    pub fn get(self, index: usize) -> Option<X11ColorName> {
        self.entries.get(index).map(x11_color_name_from_entry)
    }

    /// Iterate over the X11 color name table.
    #[must_use]
    pub fn iter(self) -> X11ColorNamesIter {
        X11ColorNamesIter {
            entries: self.entries.iter(),
        }
    }
}

impl IntoIterator for X11ColorNames {
    type Item = X11ColorName;
    type IntoIter = X11ColorNamesIter;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Iterator for X11ColorNamesIter {
    type Item = X11ColorName;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next().map(x11_color_name_from_entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}
impl DoubleEndedIterator for X11ColorNamesIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.next_back().map(x11_color_name_from_entry)
    }
}
impl ExactSizeIterator for X11ColorNamesIter {}
impl std::iter::FusedIterator for X11ColorNamesIter {}

fn x11_color_name_from_entry(entry: &ffi::ColorX11Entry) -> X11ColorName {
    let name = unsafe { CStr::from_ptr(entry.name) };
    X11ColorName {
        name,
        color: entry.color.into(),
    }
}

/// An entry in Ghostty's X11 color name table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X11ColorName {
    /// Null-terminated color name.
    pub name: &'static CStr,
    /// The RGB value of the color.
    pub color: Rgb,
}
