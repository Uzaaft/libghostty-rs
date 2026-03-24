//! Terminal screen cell and row types.
//!
//! These types represent the contents of a terminal screen.
//! A [`Cell`] is a single grid cell and a [`Row`] is a single row.
//! Both are opaque values whose fields are accessed via their methods.
use std::mem::MaybeUninit;

use crate::{
    error::{Error, Result, from_result},
    ffi,
    style::{self, PaletteIndex, RgbColor},
};

/// Represents a single terminal row.
///
/// The internal layout is opaque and must be queried via its methods.
/// Obtain cell values from terminal query APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row(ffi::GhosttyRow);

impl Row {
    fn get<T>(&self, tag: ffi::GhosttyRowData) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe { ffi::ghostty_row_get(self.0, tag, value.as_mut_ptr().cast()) };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    /// Whether this row is soft-wrapped.
    pub fn is_wrapped(self) -> Result<bool> {
        self.get(ffi::GhosttyRowData_GHOSTTY_ROW_DATA_WRAP)
    }
    /// Whether this row is a continuation of a soft-wrapped row.
    pub fn is_wrap_continuation(self) -> Result<bool> {
        self.get(ffi::GhosttyRowData_GHOSTTY_ROW_DATA_WRAP_CONTINUATION)
    }
    /// Whether any cells in this row have grapheme clusters.
    pub fn has_grapheme_cluster(self) -> Result<bool> {
        self.get(ffi::GhosttyRowData_GHOSTTY_ROW_DATA_GRAPHEME)
    }
    /// Whether any cells in this row have styling (may have false positives).
    pub fn is_styled(self) -> Result<bool> {
        self.get(ffi::GhosttyRowData_GHOSTTY_ROW_DATA_STYLED)
    }
    /// Whether any cells in this row have hyperlinks (may have false positives).
    pub fn has_hyperlink(self) -> Result<bool> {
        self.get(ffi::GhosttyRowData_GHOSTTY_ROW_DATA_HYPERLINK)
    }
    /// The semantic prompt state of this row.
    pub fn semantic_prompt(self) -> Result<RowSemanticPrompt> {
        self.get::<ffi::GhosttyRowSemanticPrompt>(
            ffi::GhosttyRowData_GHOSTTY_ROW_DATA_SEMANTIC_PROMPT,
        )
        .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }
    /// Whether this row contains a Kitty virtual placeholder.
    pub fn has_kitty_virtual_placeholder(self) -> Result<bool> {
        self.get(ffi::GhosttyRowData_GHOSTTY_ROW_DATA_KITTY_VIRTUAL_PLACEHOLDER)
    }
    /// Whether this row is dirty and requires a redraw.
    pub fn is_dirty(self) -> Result<bool> {
        self.get(ffi::GhosttyRowData_GHOSTTY_ROW_DATA_DIRTY)
    }
}

/// Represents a single terminal cell.
///
/// The internal layout is opaque and must be queried via its methods.
/// Obtain cell values from terminal query APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell(ffi::GhosttyCell);

impl Cell {
    fn get<T>(&self, tag: ffi::GhosttyCellData) -> Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result = unsafe { ffi::ghostty_cell_get(self.0, tag, value.as_mut_ptr().cast()) };
        // Since we manually model every possible query, this should never fail.
        from_result(result)?;
        // SAFETY: Value should be initialized after successful call.
        Ok(unsafe { value.assume_init() })
    }

    /// The codepoint of the cell (0 if empty or bg-color-only).
    pub fn codepoint(self) -> Result<u32> {
        self.get(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_CODEPOINT)
    }
    /// The content tag describing what kind of content is in the cell.
    pub fn content_tag(self) -> Result<CellContentTag> {
        self.get::<ffi::GhosttyCellContentTag>(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_CONTENT_TAG)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }
    /// The wide property of the cell.
    pub fn wide(self) -> Result<CellWide> {
        self.get::<ffi::GhosttyCellWide>(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_WIDE)
            .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }
    /// Whether the cell has text to render.
    pub fn has_text(self) -> Result<bool> {
        self.get(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_HAS_TEXT)
    }
    /// Whether the cell has non-default styling.
    pub fn has_styling(self) -> Result<bool> {
        self.get(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_HAS_STYLING)
    }
    /// The style ID for the cell (for use with style lookups).
    pub fn style_id(self) -> Result<style::Id> {
        self.get(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_STYLE_ID)
            .map(style::Id)
    }
    /// Whether the cell has a hyperlink.
    pub fn has_hyperlink(self) -> Result<bool> {
        self.get(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_HAS_HYPERLINK)
    }
    /// Whether the cell is protected.
    pub fn is_protected(self) -> Result<bool> {
        self.get(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_PROTECTED)
    }
    /// The semantic content type of the cell (from OSC 133).
    pub fn semantic_content(self) -> Result<CellSemanticContent> {
        self.get::<ffi::GhosttyCellSemanticContent>(
            ffi::GhosttyCellData_GHOSTTY_CELL_DATA_SEMANTIC_CONTENT,
        )
        .and_then(|v| v.try_into().map_err(|_| Error::InvalidValue))
    }

    /// The palette index for the cell's background color.
    ///
    /// Only valid when [`Cell::content_tag`] is [`CellContentTag::BgColorPalette`].
    pub fn bg_color_palette(self) -> Result<PaletteIndex> {
        self.get(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_COLOR_PALETTE)
            .map(PaletteIndex)
    }
    /// The RGB color value for the cell's background color.
    ///
    /// Only valid when [`Cell::content_tag`] is [`CellContentTag::BgColorRgb`].
    pub fn bg_color_rgb(self) -> Result<RgbColor> {
        Ok(self
            .get::<ffi::GhosttyColorRgb>(ffi::GhosttyCellData_GHOSTTY_CELL_DATA_COLOR_RGB)?
            .into())
    }
}

/// Row semantic prompt state.
///
/// Indicates whether any cells in a row are part of a shell prompt, as reported by OSC 133 sequences.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum RowSemanticPrompt {
    /// No prompt cells in this row.
    None = ffi::GhosttyRowSemanticPrompt_GHOSTTY_ROW_SEMANTIC_NONE,
    /// Prompt cells exist and this is a primary prompt line.
    Prompt = ffi::GhosttyRowSemanticPrompt_GHOSTTY_ROW_SEMANTIC_PROMPT,
    /// Prompt cells exist and this is a continuation line.
    Continuation = ffi::GhosttyRowSemanticPrompt_GHOSTTY_ROW_SEMANTIC_PROMPT_CONTINUATION,
}

/// Cell content tag.
///
/// Describes what kind of content a cell holds.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellContentTag {
    /// A single codepoint (may be zero for empty).
    Codepoint = ffi::GhosttyCellContentTag_GHOSTTY_CELL_CONTENT_CODEPOINT,
    /// A codepoint that is part of a multi-codepoint grapheme cluster.
    CodepointGrapheme = ffi::GhosttyCellContentTag_GHOSTTY_CELL_CONTENT_CODEPOINT_GRAPHEME,
    /// No text; background color from palette.
    BgColorPalette = ffi::GhosttyCellContentTag_GHOSTTY_CELL_CONTENT_BG_COLOR_PALETTE,
    /// No text; background color as RGB.
    BgColorRgb = ffi::GhosttyCellContentTag_GHOSTTY_CELL_CONTENT_BG_COLOR_RGB,
}

/// Cell wide property.
///
/// Describes the width behavior of a cell.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellWide {
    /// Not a wide character, cell width 1.
    Narrow = ffi::GhosttyCellWide_GHOSTTY_CELL_WIDE_NARROW,
    /// Wide character, cell width 2.  
    Wide = ffi::GhosttyCellWide_GHOSTTY_CELL_WIDE_WIDE,
    /// Spacer after wide character. Do not render.
    SpacerTail = ffi::GhosttyCellWide_GHOSTTY_CELL_WIDE_SPACER_TAIL,
    /// Spacer at end of soft-wrapped line for a wide character.
    SpacerHead = ffi::GhosttyCellWide_GHOSTTY_CELL_WIDE_SPACER_HEAD,
}

/// Semantic content type of a cell.
///
/// Set by semantic prompt sequences (OSC 133) to distinguish between
/// command output, user input, and shell prompt text.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum CellSemanticContent {
    /// Regular output content, such as command output.
    Output = ffi::GhosttyCellSemanticContent_GHOSTTY_CELL_SEMANTIC_OUTPUT,
    /// Content that is part of user input.
    Input = ffi::GhosttyCellSemanticContent_GHOSTTY_CELL_SEMANTIC_INPUT,
    /// Content that is part of a shell prompt.
    Prompt = ffi::GhosttyCellSemanticContent_GHOSTTY_CELL_SEMANTIC_PROMPT,
}
