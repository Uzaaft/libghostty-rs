//! Pack DEC and ANSI mode numbers and encode DECRPM reports.
//!
//! A [`Mode`] is a terminal mode number packed into 16 bits, together with
//! whether it's an ANSI mode or a DEC private mode.
//!
//! The number alone isn't enough. ANSI mode 4 is insert mode, and DEC private
//! mode 4 is slow scroll. Use these to name a mode when you read or set one
//! on a terminal with [`Terminal::set_mode`](super::Terminal::set_mode), or to
//! encode a DECRPM report with [`Mode::encode_report`]. You don't need a
//! terminal to pack one.
use crate::{
    error::{Result, from_result_with_len},
    ffi,
};

/// A terminal mode consisting of its value and its kind (DEC/ANSI).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Mode(pub(crate) ffi::Mode);

impl Mode {
    const ANSI_BIT: u16 = 1 << 15;

    /// Create a new mode from its numeric value and its kind.
    #[must_use]
    pub const fn new(v: u16, kind: Kind) -> Self {
        match kind {
            Kind::Ansi => Self(v | Self::ANSI_BIT),
            Kind::Dec => Self(v),
        }
    }

    /// The numeric value of the mode.
    #[must_use]
    pub const fn value(self) -> u16 {
        (self.0) & 0x7fff
    }

    /// The kind of the mode (DEC/ANSI).
    #[must_use]
    pub const fn kind(self) -> Kind {
        if (self.0) & Self::ANSI_BIT > 0 {
            Kind::Ansi
        } else {
            Kind::Dec
        }
    }

    /// Encode a DECRPM (DEC Private Mode Report) response sequence.
    ///
    /// Writes a mode report escape sequence into the provided buffer.
    /// The generated sequence has the form:
    /// - DEC private mode: `CSI ? Ps1 ; Ps2 $ y`
    /// - ANSI mode: `CSI Ps1 ; Ps2 $ y`
    ///
    /// If the buffer is too small, returns
    /// [`Error::OutOfSpace`](crate::error::Error::OutOfSpace) with the
    /// required buffer size. The caller can then retry with a sufficiently
    /// sized buffer.
    ///
    /// ```
    /// use libghostty_vt::terminal::mode::{Mode, ReportState};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut buf = [0u8; 32];
    ///
    /// // Report DECCKM (DEC private mode 1) as set.
    /// let len = Mode::DECCKM.encode_report(ReportState::Set, &mut buf)?;
    /// assert_eq!(&buf[..len], b"\x1b[?1;1$y");
    /// # Ok(())
    /// # }
    /// ```
    pub fn encode_report(self, state: ReportState, buf: &mut [u8]) -> Result<usize> {
        let mut written = 0;
        // SAFETY: The pointer and length describe `buf`, a valid writable
        // region, and upstream only writes within `buf_len`, even when it
        // runs out of space partway through (an empty `buf` is fine too:
        // upstream accepts a zero-length or NULL buffer as a size query).
        let result = unsafe {
            ffi::ghostty_mode_report_encode(
                self.0,
                state.into(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &raw mut written,
            )
        };
        // `written` doubles as the required size on `OUT_OF_SPACE`, so it
        // must be threaded into the error rather than dropped.
        from_result_with_len(result, written)
    }

    /// Keyboard action (disable keyboard)
    pub const KAM: Self = Self::new(2, Kind::Ansi);
    /// Insert mode
    pub const INSERT: Self = Self::new(4, Kind::Ansi);
    /// Send/receive mode
    pub const SRM: Self = Self::new(12, Kind::Ansi);
    /// Linefeed/new line mode
    pub const LINEFEED: Self = Self::new(20, Kind::Ansi);

    /// Cursor keys
    pub const DECCKM: Self = Self::new(1, Kind::Dec);
    /// 132/80 column mode
    pub const _132_COLUMN: Self = Self::new(3, Kind::Dec);
    /// Slow scroll
    pub const SLOW_SCROLL: Self = Self::new(4, Kind::Dec);
    /// Reverse video
    pub const REVERSE_COLORS: Self = Self::new(5, Kind::Dec);
    /// Origin mode
    pub const ORIGIN: Self = Self::new(6, Kind::Dec);
    /// Auto-wrap mode
    pub const WRAPAROUND: Self = Self::new(7, Kind::Dec);
    /// Auto-repeat keys
    pub const AUTOREPEAT: Self = Self::new(8, Kind::Dec);
    /// X10 mouse reporting
    pub const X10_MOUSE: Self = Self::new(9, Kind::Dec);
    /// Cursor blink
    pub const CURSOR_BLINKING: Self = Self::new(12, Kind::Dec);
    /// Cursor visible (DECTCEM)
    pub const CURSOR_VISIBLE: Self = Self::new(25, Kind::Dec);
    /// Allow 132 column mode
    pub const ENABLE_MODE3: Self = Self::new(40, Kind::Dec);
    /// Reverse wrap
    pub const REVERSE_WRAP: Self = Self::new(45, Kind::Dec);
    /// Alternate screen (legacy)
    pub const ALT_SCREEN_LEGACY: Self = Self::new(47, Kind::Dec);
    /// Application keypad
    pub const KEYPAD_KEYS: Self = Self::new(66, Kind::Dec);
    /// Backarrow key mode (DECBKM)
    pub const BACKARROW_KEY_MODE: Self = Self::new(67, Kind::Dec);
    /// Left/right margin mode
    pub const LEFT_RIGHT_MARGIN: Self = Self::new(69, Kind::Dec);
    /// Normal mouse tracking
    pub const NORMAL_MOUSE: Self = Self::new(1000, Kind::Dec);
    /// Button-event mouse tracking
    pub const BUTTON_MOUSE: Self = Self::new(1002, Kind::Dec);
    /// Any-event mouse tracking
    pub const ANY_MOUSE: Self = Self::new(1003, Kind::Dec);
    /// Focus in/out events
    pub const FOCUS_EVENT: Self = Self::new(1004, Kind::Dec);
    /// UTF-8 mouse format
    pub const UTF8_MOUSE: Self = Self::new(1005, Kind::Dec);
    /// SGR mouse format
    pub const SGR_MOUSE: Self = Self::new(1006, Kind::Dec);
    /// Alternate scroll mode
    pub const ALT_SCROLL: Self = Self::new(1007, Kind::Dec);
    /// `URxvt` mouse format
    pub const URXVT_MOUSE: Self = Self::new(1015, Kind::Dec);
    /// SGR-Pixels mouse format
    pub const SGR_PIXELS_MOUSE: Self = Self::new(1016, Kind::Dec);
    /// Ignore keypad with `NumLock`
    pub const NUMLOCK_KEYPAD: Self = Self::new(1035, Kind::Dec);
    /// Alt key sends ESC prefix
    pub const ALT_ESC_PREFIX: Self = Self::new(1036, Kind::Dec);
    /// Alt sends escape
    pub const ALT_SENDS_ESC: Self = Self::new(1039, Kind::Dec);
    /// Extended reverse wrap
    pub const REVERSE_WRAP_EXT: Self = Self::new(1045, Kind::Dec);
    /// Alternate screen
    pub const ALT_SCREEN: Self = Self::new(1047, Kind::Dec);
    /// Save cursor (DECSC)
    pub const SAVE_CURSOR: Self = Self::new(1048, Kind::Dec);
    /// Alt screen + save cursor + clear
    pub const ALT_SCREEN_SAVE: Self = Self::new(1049, Kind::Dec);
    /// Bracketed paste mode
    pub const BRACKETED_PASTE: Self = Self::new(2004, Kind::Dec);
    /// Synchronized output
    pub const SYNC_OUTPUT: Self = Self::new(2026, Kind::Dec);
    /// Grapheme cluster mode
    pub const GRAPHEME_CLUSTER: Self = Self::new(2027, Kind::Dec);
    /// Report color scheme
    pub const COLOR_SCHEME_REPORT: Self = Self::new(2031, Kind::Dec);
    /// Report terminal visibility
    pub const VISIBILITY_REPORT: Self = Self::new(2033, Kind::Dec);
    /// In-band size reports
    pub const IN_BAND_RESIZE: Self = Self::new(2048, Kind::Dec);
    /// Kitty clipboard protocol paste events
    pub const PASTE_EVENTS: Self = Self::new(5522, Kind::Dec);
}
impl From<Mode> for ffi::Mode {
    fn from(value: Mode) -> Self {
        value.0
    }
}

/// The kind of a terminal mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// DEC terminal mode.
    Dec,
    /// ANSI terminal mode.
    Ansi,
}

/// DECRPM report state values.
///
/// These correspond to the Ps2 parameter in a DECRPM response
/// sequence (`CSI ? Ps1 ; Ps2 $ y`).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum ReportState {
    /// Mode is not recognized
    NotRecognized = ffi::ModeReportState::NOT_RECOGNIZED,
    /// Mode is set (enabled)
    Set = ffi::ModeReportState::SET,
    /// Mode is reset (disabled)
    Reset = ffi::ModeReportState::RESET,
    /// Mode is permanently set
    PermanentlySet = ffi::ModeReportState::PERMANENTLY_SET,
    /// Mode is permanently reset
    PermanentlyReset = ffi::ModeReportState::PERMANENTLY_RESET,
}
