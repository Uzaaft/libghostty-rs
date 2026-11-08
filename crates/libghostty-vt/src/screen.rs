//! Terminal screen cell and row types.
//!
//! These types represent the contents of a terminal screen.
//! A [`Cell`] is a single grid cell and a [`Row`] is a single row.
//! Both are opaque values whose fields are accessed via their methods.
use std::mem::MaybeUninit;

use crate::{
    color::{PaletteIndex, Rgb},
    error::{Error, Result, from_result},
    ffi, style,
};

/// Terminal screen identifier.
///
/// Identifies which screen buffer is active in the terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, int_enum::IntEnum)]
#[repr(i32)]
pub enum Screen {
    /// The primary (normal) screen.
    #[default]
    Primary = ffi::TerminalScreen::PRIMARY,
    /// The alternate screen.
    Alternate = ffi::TerminalScreen::ALTERNATE,
}
/// Represents a single terminal row.
///
/// The internal layout is opaque and must be queried via its methods.
/// Obtain cell values from terminal query APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row(pub(crate) ffi::Row);

impl Row {
    fn get<T>(&self, tag: ffi::RowData::Type) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe { ffi::ghostty_row_get(self.0, tag, value.as_mut_ptr().cast()) };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    /// Whether this row is soft-wrapped.
    pub fn is_wrapped(self) -> Result<bool> {
        self.get(ffi::RowData::WRAP)
    }
    /// Whether this row is a continuation of a soft-wrapped row.
    pub fn is_wrap_continuation(self) -> Result<bool> {
        self.get(ffi::RowData::WRAP_CONTINUATION)
    }
    /// Whether any cells in this row have grapheme clusters.
    pub fn has_grapheme_cluster(self) -> Result<bool> {
        self.get(ffi::RowData::GRAPHEME)
    }
    /// Whether any cells in this row have styling (may have false positives).
    pub fn is_styled(self) -> Result<bool> {
        self.get(ffi::RowData::STYLED)
    }
    /// Whether any cells in this row have hyperlinks (may have false positives).
    pub fn has_hyperlink(self) -> Result<bool> {
        self.get(ffi::RowData::HYPERLINK)
    }
    /// The semantic prompt state of this row.
    pub fn semantic_prompt(self) -> Result<RowSemanticPrompt> {
        self.get::<ffi::RowSemanticPrompt::Type>(ffi::RowData::SEMANTIC_PROMPT)
            .and_then(|v| v.try_into().map_err(|_| Error::Unsupported))
    }
    /// Whether this row contains a Kitty virtual placeholder.
    pub fn has_kitty_virtual_placeholder(self) -> Result<bool> {
        self.get(ffi::RowData::KITTY_VIRTUAL_PLACEHOLDER)
    }
    /// Whether this row is dirty and requires a redraw.
    pub fn is_dirty(self) -> Result<bool> {
        self.get(ffi::RowData::DIRTY)
    }
}

/// Represents a single terminal cell.
///
/// The internal layout is opaque and must be queried via its methods.
/// Obtain cell values from terminal query APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell(pub(crate) ffi::Cell);

/// Typed keys for single and batched cell queries.
pub mod cell {
    use super::{
        Cell, CellContentTag, CellSemanticContent, CellWide, Error, MaybeUninit, PaletteIndex,
        Result, Rgb, ffi, from_result, style,
    };

    /// A cell data key with an associated result type.
    ///
    /// This trait is sealed: only the keys provided by this module implement it.
    pub trait Key: private::Key {
        /// The Rust value returned for this key.
        type Value;
    }

    /// A tuple of cell data keys, in result order.
    ///
    /// Implemented for tuples of one through twelve keys. Keys may be repeated.
    /// This trait is sealed.
    pub trait Query: private::Query {
        /// The tuple of Rust values returned for these keys.
        type Output;
    }

    pub(super) mod private {
        use super::{Cell, Result, ffi};

        // Sealing protects the correspondence between a C key and its output
        // type. Raw enums stay integers until their checked Rust conversion.
        pub trait Key {
            type Raw;
            const TAG: ffi::CellData::Type;
            fn decode(raw: Self::Raw) -> Result<<Self as super::Key>::Value>
            where
                Self: super::Key;
        }

        pub trait Query {
            fn read(cell: Cell) -> Result<<Self as super::Query>::Output>
            where
                Self: super::Query;
        }
    }

    // Define each tag, raw output type and conversion together, so named
    // getters and tuple queries cannot disagree about how to read a key.
    macro_rules! keys {
        ($($(#[$doc:meta])* $name:ident: $tag:ident, $raw:ty => $value:ty, $decode:expr;)*) => {$(
            $(#[$doc])*
            #[derive(Clone, Copy, Debug)]
            pub struct $name;

            impl Key for $name {
                type Value = $value;
            }

            impl private::Key for $name {
                type Raw = $raw;
                const TAG: ffi::CellData::Type = ffi::CellData::$tag;
                fn decode(raw: Self::Raw) -> Result<<Self as Key>::Value> {
                    ($decode)(raw)
                }
            }
        )*};
    }

    keys! {
        /// The codepoint of the cell (0 if empty or bg-color-only).
        Codepoint: CODEPOINT, u32 => u32, Ok;
        /// The content tag describing what kind of content is in the cell.
        ContentTag: CONTENT_TAG, ffi::CellContentTag::Type => CellContentTag,
            |v: ffi::CellContentTag::Type| v.try_into().map_err(|_| Error::Unsupported);
        /// The wide property of the cell.
        Wide: WIDE, ffi::CellWide::Type => CellWide,
            |v: ffi::CellWide::Type| v.try_into().map_err(|_| Error::Unsupported);
        /// Whether the cell has text to render.
        HasText: HAS_TEXT, bool => bool, Ok;
        /// Whether the cell has non-default styling.
        HasStyling: HAS_STYLING, bool => bool, Ok;
        /// The style ID for the cell (for use with style lookups).
        StyleId: STYLE_ID, ffi::StyleId => style::Id, |v| Ok(style::Id(v));
        /// Whether the cell has a hyperlink.
        HasHyperlink: HAS_HYPERLINK, bool => bool, Ok;
        /// Whether the cell is protected.
        Protected: PROTECTED, bool => bool, Ok;
        /// The semantic content type of the cell (from OSC 133).
        SemanticContent: SEMANTIC_CONTENT, ffi::CellSemanticContent::Type => CellSemanticContent,
            |v: ffi::CellSemanticContent::Type| v.try_into().map_err(|_| Error::Unsupported);
        /// The palette index for the cell's background color.
        ///
        /// Only valid when the content tag is [`CellContentTag::BgColorPalette`].
        ColorPalette: COLOR_PALETTE, ffi::ColorPaletteIndex => PaletteIndex, |v| Ok(PaletteIndex(v));
        /// The RGB value for the cell's background color.
        ///
        /// Only valid when the content tag is [`CellContentTag::BgColorRgb`].
        ColorRgb: COLOR_RGB, ffi::ColorRgb => Rgb, |v: ffi::ColorRgb| Ok(v.into());
    }

    // Rust has no variadic generics. Each implementation owns one correctly
    // typed stack slot per key and passes their addresses in tuple order.
    // The recursive arm generates all shorter tuples as well.
    macro_rules! queries {
        ($first:ident $(, $rest:ident)*) => {
            impl<$first: Key, $($rest: Key),*> Query for ($first, $($rest,)*) {
                type Output = ($first::Value, $($rest::Value,)*);
            }

            impl<$first: Key, $($rest: Key),*> private::Query for ($first, $($rest,)*) {
                #[expect(non_snake_case, reason = "use each key type's name for its output slot")]
                fn read(cell: Cell) -> Result<<Self as Query>::Output> {
                    let mut $first = MaybeUninit::<$first::Raw>::zeroed();
                    $(let mut $rest = MaybeUninit::<$rest::Raw>::zeroed();)*
                    let keys = [$first::TAG, $($rest::TAG,)*];
                    let mut values = [$first.as_mut_ptr().cast(), $($rest.as_mut_ptr().cast(),)*];
                    // SAFETY: Sealed keys pair each tag with the documented
                    // raw type. Every output points to a distinct live slot.
                    from_result(unsafe {
                        ffi::ghostty_cell_get_multi(
                            cell.0, keys.len(), keys.as_ptr(), values.as_mut_ptr(),
                            std::ptr::null_mut(),
                        )
                    })?;
                    // SAFETY: Every output is initialized after a successful
                    // batch. Conversion checks raw enum discriminants.
                    unsafe {
                        Ok(($first::decode($first.assume_init())?,
                            $($rest::decode($rest.assume_init())?,)*))
                    }
                }
            }
            queries!($($rest),*);
        };
        () => {};
    }

    queries!(A, B, C, D, E, F, G, H, I, J, K, L);
}

impl Cell {
    /// Get cell data using a typed key.
    ///
    /// The key determines the result type. For example, [`cell::Codepoint`]
    /// returns a `u32`, while [`cell::HasText`] returns a `bool`.
    /// The named getters, such as [`Self::codepoint`], use the same keys.
    pub fn get<K: cell::Key>(self, _key: K) -> Result<K::Value> {
        let mut value = MaybeUninit::<K::Raw>::zeroed();
        // SAFETY: The sealed key pairs its tag with the documented raw type.
        let result = unsafe { ffi::ghostty_cell_get(self.0, K::TAG, value.as_mut_ptr().cast()) };
        from_result(result)?;
        // SAFETY: The sealed key specifies the documented raw output type,
        // and the successful call initialized it.
        K::decode(unsafe { value.assume_init() })
    }

    /// Get multiple data fields from a cell in a single call.
    ///
    /// Use this when each call into the library is expensive, for example from
    /// WebAssembly or from another language through its foreign function
    /// interface. It reads every key in one call. From C in the same process it
    /// costs the same as calling `ghostty_cell_get` once per key, so use
    /// whichever reads better.
    ///
    /// Returns the values in key order. Processing stops at the first error;
    /// no partial result is returned. The batch uses no heap allocation.
    ///
    /// ```
    /// use libghostty_vt::{Terminal, screen::cell, terminal::{Point, PointCoordinate}};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut terminal = Terminal::new(80, 24)?;
    /// terminal.vt_write(b"A");
    /// let cell = terminal.grid_ref(Point::Viewport(PointCoordinate { x: 0, y: 0 }))?.cell()?;
    /// let (codepoint, has_text) = cell.get_multi((cell::Codepoint, cell::HasText))?;
    /// assert_eq!(codepoint, u32::from('A'));
    /// assert!(has_text);
    /// # Ok(()) }
    /// ```
    pub fn get_multi<Q: cell::Query>(self, _keys: Q) -> Result<Q::Output> {
        Q::read(self)
    }

    /// The codepoint of the cell (0 if empty or bg-color-only).
    pub fn codepoint(self) -> Result<u32> {
        self.get(cell::Codepoint)
    }
    /// The content tag describing what kind of content is in the cell.
    pub fn content_tag(self) -> Result<CellContentTag> {
        self.get(cell::ContentTag)
    }
    /// The wide property of the cell.
    pub fn wide(self) -> Result<CellWide> {
        self.get(cell::Wide)
    }
    /// Whether the cell has text to render.
    pub fn has_text(self) -> Result<bool> {
        self.get(cell::HasText)
    }
    /// Whether the cell has non-default styling.
    pub fn has_styling(self) -> Result<bool> {
        self.get(cell::HasStyling)
    }
    /// The style ID for the cell (for use with style lookups).
    pub fn style_id(self) -> Result<style::Id> {
        self.get(cell::StyleId)
    }
    /// Whether the cell has a hyperlink.
    pub fn has_hyperlink(self) -> Result<bool> {
        self.get(cell::HasHyperlink)
    }
    /// Whether the cell is protected.
    pub fn is_protected(self) -> Result<bool> {
        self.get(cell::Protected)
    }
    /// The semantic content type of the cell (from OSC 133).
    pub fn semantic_content(self) -> Result<CellSemanticContent> {
        self.get(cell::SemanticContent)
    }

    /// The palette index for the cell's background color.
    ///
    /// Only valid when [`Cell::content_tag`] is [`CellContentTag::BgColorPalette`].
    pub fn bg_color_palette(self) -> Result<PaletteIndex> {
        self.get(cell::ColorPalette)
    }
    /// The RGB color value for the cell's background color.
    ///
    /// Only valid when [`Cell::content_tag`] is [`CellContentTag::BgColorRgb`].
    pub fn bg_color_rgb(self) -> Result<Rgb> {
        self.get(cell::ColorRgb)
    }
}

/// Row semantic prompt state.
///
/// Indicates whether any cells in a row are part of a shell prompt, as reported by OSC 133 sequences.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum RowSemanticPrompt {
    /// No prompt cells in this row.
    None = ffi::RowSemanticPrompt::NONE,
    /// Prompt cells exist and this is a primary prompt line.
    Prompt = ffi::RowSemanticPrompt::PROMPT,
    /// Prompt cells exist and this is a continuation line.
    Continuation = ffi::RowSemanticPrompt::PROMPT_CONTINUATION,
}

/// Cell content tag.
///
/// Describes what kind of content a cell holds.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellContentTag {
    /// A single codepoint (may be zero for empty).
    Codepoint = ffi::CellContentTag::CODEPOINT,
    /// A codepoint that is part of a multi-codepoint grapheme cluster.
    CodepointGrapheme = ffi::CellContentTag::CODEPOINT_GRAPHEME,
    /// No text; background color from palette.
    BgColorPalette = ffi::CellContentTag::BG_COLOR_PALETTE,
    /// No text; background color as RGB.
    BgColorRgb = ffi::CellContentTag::BG_COLOR_RGB,
}

/// Cell wide property.
///
/// Describes the width behavior of a cell.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellWide {
    /// Not a wide character, cell width 1.
    Narrow = ffi::CellWide::NARROW,
    /// Wide character, cell width 2.  
    Wide = ffi::CellWide::WIDE,
    /// Spacer after wide character. Do not render.
    SpacerTail = ffi::CellWide::SPACER_TAIL,
    /// Spacer at end of soft-wrapped line for a wide character.
    SpacerHead = ffi::CellWide::SPACER_HEAD,
}

/// Semantic content type of a cell.
///
/// Set by semantic prompt sequences (OSC 133) to distinguish between
/// command output, user input, and shell prompt text.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellSemanticContent {
    /// Regular output content, such as command output.
    Output = ffi::CellSemanticContent::OUTPUT,
    /// Content that is part of user input.
    Input = ffi::CellSemanticContent::INPUT,
    /// Content that is part of a shell prompt.
    Prompt = ffi::CellSemanticContent::PROMPT,
}
