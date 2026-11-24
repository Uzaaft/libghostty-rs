//! Measure codepoints and graphemes the way the terminal does.
//!
//! These measure text the same way the terminal does, with the same width
//! table and the same grapheme cluster rules.
//!
//! Use them to predict the layout of text you haven't written to the terminal,
//! such as an IME preedit overlay. [`codepoint_width`] measures one codepoint.
//! [`grapheme_width`] finds and measures a full grapheme cluster, with the
//! rules the terminal uses when mode 2027 is on. You don't need a terminal.
//!
//! We don't have a guide or an example for these yet.
//!
//! ```rust
//! let width = libghostty_vt::unicode::codepoint_width('\u{1F600}');
//! ```

use crate::ffi;

/// Returns the terminal display width of a Unicode codepoint in terminal grid
/// cells: 0, 1, or 2.
///
/// This is the same width table the terminal itself uses when laying out
/// printed text, so callers can predict column layout that exactly matches what
/// the terminal will do when the text is actually written to it.
///
/// Semantics:
///
///   - Returns 0 for zero-width codepoints: C0/C1 control characters,
///     nonspacing and enclosing combining marks, default-ignorable codepoints
///     (ZWJ, ZWNJ, variation selectors, etc.), and surrogate codepoints,
///   - Returns 2 for wide characters: East Asian Wide/Fullwidth (including
///     emoji with default emoji presentation) and regional indicators. Width
///     is clamped to 2 (e.g. the three-em dash).
///   - Returns 1 for everything else.
///
/// This operates on a single character only and therefore
/// cannot account for grapheme-cluster-level width rules. For cluster-accurate
/// widths, use [`grapheme_width`]. Summing per-character widths is only correct
/// when mode 2027, grapheme clustering, is disabled.
///
/// This function is pure, allocates nothing, and is thread-safe.
#[must_use]
pub fn codepoint_width(codepoint: char) -> u8 {
    crate::sys::GATE.freeze();
    unsafe { ffi::ghostty_unicode_codepoint_width(u32::from(codepoint)) }
}

/// Measures the terminal display width of the first grapheme cluster in a sequence.
///
/// This uses the exact same grapheme segmentation and cluster width rules the
/// terminal itself uses when printing text with grapheme clustering enabled,
/// mode 2027, so callers can predict column layout that exactly matches what
/// the terminal will do when the text is actually written to it. Unlike
/// [`codepoint_width`], this accounts for cluster-level rules: emoji variation
/// selectors, ZWJ sequences, combining marks, and skin tone modifiers.
///
/// Reads codepoints from cps until the terminal would consider the grapheme
/// cluster complete, and returns two items in a tuple: first the total number
/// of codepoints consumed, and second the cluster's total width in cells
/// (0, 1, or 2). Codepoints consumed is 0 if and only if len is 0; otherwise
/// consumes at least one codepoint.
///
/// Measure a whole string by calling in a loop:
///
/// ```rust
/// use libghostty_vt::unicode;
///
/// let mut total = 0usize;
/// let mut i = 0usize;
///
/// let codepoints = ['â', '辣', 'غ'];
/// while i < codepoints.len() {
///     let (consumed, width) = unicode::grapheme_width(&codepoints[i..]);
///     total += usize::from(width);
///     i += consumed;
/// }
///
/// assert_eq!(total, 4);
/// assert_eq!(i, 3);
/// ```
///
/// This is not a streaming API. The provided sequence must contain a complete
/// first grapheme cluster, or the logical end of the string. If input arrives in
/// chunks, keep buffering while this function consumes all available characters
/// and the stream may still continue; a later character could still extend the
/// cluster and change its width.
///
/// Mode dependence: this models mode 2027, grapheme clustering, enabled. When
/// mode 2027 is disabled, clusters never combine and variation selectors never
/// change width; predict layout in that case by summing [`codepoint_width`] over
/// each character instead.
#[must_use]
pub fn grapheme_width(chars: &[char]) -> (usize, u8) {
    crate::sys::GATE.freeze();

    let mut width = 0;
    let consumed = unsafe {
        // Note that it is always safe to cast &[char] to &[u32], as long
        // as the slice is never mutated.
        ffi::ghostty_unicode_grapheme_width(chars.as_ptr().cast(), chars.len(), &raw mut width)
    };
    (consumed, width)
}
