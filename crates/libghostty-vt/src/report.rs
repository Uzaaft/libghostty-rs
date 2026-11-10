//! Reports size, color scheme, etc. information to the running program.
//!
//! # Size Reports
//!
//! A size report tells a program how big your terminal is, in cells and pixels.
//!
//! These answer in-band size reports (mode 2048) and XTWINOPS queries
//! (`CSI 14 t`, `CSI 16 t`, `CSI 18 t`, `CSI 19 t`). A terminal answers those
//! for you through a [`Terminal::on_size`](crate::Terminal::on_size), so you
//! only need this without one.
//!
//! Fill in a [`SizeReportSize`] and pass it to [`SizeReport::encode`].
//!
//! # Color Scheme Reports
//!
//! A color scheme report tells a program whether your terminal is light or dark.
//!
//! You need this when a program sets mode 2031 and expects to hear when the
//! scheme changes. Encode the report and write it to the pty yourself. You don't
//! need it to answer a `CSI ? 996 n` query. A terminal does that for you through
//! [`Terminal::on_color_scheme`](crate::Terminal::on_color_scheme).
use crate::{
    error::{Result, from_result_with_len},
    ffi,
};

#[doc(inline)]
pub use ffi::SizeReportSize;

/// Size report style.
///
/// Determines the output format for the terminal size report.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum SizeReport {
    /// In-band size report (mode 2048): `ESC [ 48 ; rows ; cols ; height ; width t`
    Mode2048 = ffi::SizeReportStyle::MODE_2048,
    /// XTWINOPS text area size in pixels: `ESC [ 4 ; height ; width t`
    Csi14T = ffi::SizeReportStyle::CSI_14_T,
    /// XTWINOPS cell size in pixels: `ESC [ 6 ; height ; width t`
    Csi16T = ffi::SizeReportStyle::CSI_16_T,
    /// XTWINOPS text area size in characters: `ESC [ 8 ; rows ; cols t`
    Csi18T = ffi::SizeReportStyle::CSI_18_T,
    /// XTWINOPS screen size in characters: `ESC [ 9 ; rows ; cols t`
    ///
    /// The physical screen size is not known, so this reports the text
    /// area size in characters, the same as CSI 18 t.
    Csi19T = ffi::SizeReportStyle::CSI_19_T,
}

impl SizeReport {
    /// Encode a terminal size report into an escape sequence.
    ///
    /// Encodes a size report in the format specified by `self` into the
    /// provided buffer.
    ///
    /// If the buffer is too small, returns
    /// [`Error::OutOfSpace`](crate::error::Error::OutOfSpace) with the
    /// required buffer size. The caller can then retry with a sufficiently
    /// sized buffer.
    ///
    /// ```
    /// use libghostty_vt::report::{SizeReport, SizeReportSize};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let size = SizeReportSize {
    ///     rows: 24,
    ///     columns: 80,
    ///     cell_width: 9,
    ///     cell_height: 18,
    /// };
    /// let mut buf = [0u8; 32];
    ///
    /// // Answer an XTWINOPS text area size query (CSI 18 t).
    /// let len = SizeReport::Csi18T.encode(&size, &mut buf)?;
    /// assert_eq!(&buf[..len], b"\x1b[8;24;80t");
    /// # Ok(())
    /// # }
    /// ```
    pub fn encode(self, size: &SizeReportSize, buf: &mut [u8]) -> Result<usize> {
        let mut written = 0;
        // SAFETY: The pointer and length describe `buf`, a valid writable
        // region, and upstream only writes within `buf_len`, even when it
        // runs out of space partway through (an empty `buf` is fine too:
        // upstream accepts a zero-length or NULL buffer as a size query).
        // `size` remains borrowed for the duration of the call.
        let result = unsafe {
            ffi::ghostty_size_report_encode(
                self.into(),
                size,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &raw mut written,
            )
        };
        // `written` doubles as the required size on `OUT_OF_SPACE`, so it
        // must be threaded into the error rather than dropped.
        from_result_with_len(result, written)
    }
}

/// Color scheme reported in response to a CSI ? 996 n query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
#[expect(missing_docs, reason = "self-explanatory")]
pub enum ColorScheme {
    Light = ffi::ColorScheme::LIGHT,
    Dark = ffi::ColorScheme::DARK,
}

impl ColorScheme {
    /// Encode a color scheme report into an escape sequence.
    ///
    /// Encodes a color scheme report into the provided buffer. Dark color
    /// schemes emit `ESC [ ? 997 ; 1 n`, and light color schemes emit
    /// `ESC [ ? 997 ; 2 n`. The encoded bytes are identical to the terminal's
    /// internal `CSI ? 996 n` query response.
    ///
    /// Hosts should gate unsolicited sends on mode 2031 being set, which can
    /// be checked via the mode getters.
    ///
    /// If the buffer is too small, returns
    /// [`Error::OutOfSpace`](crate::error::Error::OutOfSpace) with the
    /// required buffer size. The caller can then retry with a sufficiently
    /// sized buffer.
    pub fn encode(self, buf: &mut [u8]) -> Result<usize> {
        let mut written = 0;
        let result = unsafe {
            ffi::ghostty_color_scheme_report_encode(
                self.into(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &raw mut written,
            )
        };
        from_result_with_len(result, written)
    }
}

impl From<ColorScheme> for ffi::ColorScheme::Type {
    fn from(value: ColorScheme) -> Self {
        match value {
            ColorScheme::Light => ffi::ColorScheme::LIGHT,
            ColorScheme::Dark => ffi::ColorScheme::DARK,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    /// Mirrors upstream's size report tests. Rows and columns differ, as do
    /// cell width and height, so swapping either pair (or text-area pixels
    /// for cell pixels) changes the output.
    const SIZE_REPORT_SIZE: SizeReportSize = SizeReportSize {
        rows: 24,
        columns: 80,
        cell_width: 9,
        cell_height: 18,
    };

    /// Each style follows its form in the header. Mode 2048 and CSI 14 t
    /// report the text area in pixels (rows * cell height = 432, columns *
    /// cell width = 720), while CSI 16 t reports a single cell.
    #[test]
    fn size_report_encode_report() {
        let mut buf = [0u8; 32];
        for (style, expected) in [
            (SizeReport::Mode2048, b"\x1b[48;24;80;432;720t".as_slice()),
            (SizeReport::Csi14T, b"\x1b[4;432;720t"),
            (SizeReport::Csi16T, b"\x1b[6;18;9t"),
            (SizeReport::Csi18T, b"\x1b[8;24;80t"),
            (SizeReport::Csi19T, b"\x1b[9;24;80t"),
        ] {
            let len = style.encode(&SIZE_REPORT_SIZE, &mut buf).unwrap();
            assert_eq!(&buf[..len], expected, "{style:?}");
        }
    }

    /// A short (or empty) buffer reports how much space the sequence needs,
    /// so callers can retry with a big enough one.
    #[test]
    fn size_report_encode_report_out_of_space() {
        let result = SizeReport::Mode2048.encode(&SIZE_REPORT_SIZE, &mut [0u8; 4]);
        assert!(matches!(result, Err(Error::OutOfSpace { required: 19 })));

        let result = SizeReport::Mode2048.encode(&SIZE_REPORT_SIZE, &mut []);
        assert!(matches!(result, Err(Error::OutOfSpace { required: 19 })));
    }
}
