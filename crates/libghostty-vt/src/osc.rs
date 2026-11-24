//! Parse OSC sequences into commands.
//!
//! The OSC parser turns one OSC (Operating System Command) sequence into a
//! command you can read.
//!
//! A terminal parses these by itself, so you only need this for OSC sequences
//! outside of one. You feed it the bytes between `ESC ]` and the terminator, one
//! at a time. A sequence can arrive in pieces across many reads, and you don't
//! have to buffer it.
//!
//! To handle sequences in a terminal, see the [Custom Sequences] guide.
//!
//! # Example
//!
//! ```rust
//! # use libghostty_vt::error::Error;
//! use libghostty_vt::osc;
//!
//! # fn main() -> Result<(), Error> {
//! let mut parser = osc::Parser::new()?;
//!
//! for &b in b"0;hello" {
//!     parser.next_byte(b);
//! }
//!
//! // This sequence ended with BEL (0x07).
//! let command = parser.end(0x07);
//! if let osc::CommandType::ChangeWindowTitle { title } = command.command_type() {
//!     println!("Title: {}", title.escape_ascii());
//! }
//!
//! // Reset before the next sequence.
//! parser.reset();
//! # Ok(()) }
//! ```
//!
#![doc = crate::guide_ref!("Custom Sequences", "custom-sequences")]
use std::{marker::PhantomData, mem::MaybeUninit};

use crate::{
    alloc::{Allocator, Object},
    error::{Result, from_result},
    ffi,
};

/// Opaque handle to an OSC parser instance.
///
/// This handle represents an OSC (Operating System Command) parser that can be
/// used to parse the contents of OSC sequences.
#[derive(Debug)]
pub struct Parser<'alloc>(Object<'alloc, ffi::OscParserImpl>);

impl<'alloc> Parser<'alloc> {
    /// Create a new OSC parser.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new OSC parser with a custom allocator.
    ///
    /// See [Memory] regarding custom memory management and lifetimes.
    ///
    #[doc = crate::book_ref!("Memory", "memory")]
    pub fn new_in<'ctx: 'alloc>(alloc: &'alloc Allocator<'ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::Allocator) -> Result<Self> {
        crate::sys::GATE.freeze();

        let mut raw: ffi::OscParser = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_osc_new(alloc, &raw mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    /// Reset an OSC parser instance to its initial state.
    ///
    /// Resets the parser state, clearing any partially parsed OSC sequences
    /// and returning the parser to its initial state. This is useful for
    /// reusing a parser instance or recovering from parse errors.
    pub fn reset(&mut self) {
        crate::sys::GATE.freeze();
        unsafe { ffi::ghostty_osc_reset(self.0.as_raw()) }
    }

    /// Set the most bytes to keep from each OSC sequence whose number the
    /// parser does not implement.
    ///
    /// Every OSC sequence starts with a number that says what it is, such as
    /// 2 for the window title or 8 for a hyperlink. Set this to implement a
    /// sequence yourself, and read its raw bytes with [`CommandType::Unknown`].
    ///
    /// Zero, the default, discards these sequences and they produce
    /// [`CommandType::Invalid`]. Any other value makes them produce
    /// [`CommandType::Unknown`].
    ///
    /// A sequence longer than the limit is still reported. Its content holds
    /// the first bytes up to the limit, and its `truncated` field is true.
    ///
    /// Limits up to 2048 bytes use a buffer the parser already owns and never
    /// allocate memory. Larger limits allocate memory from the parser's
    /// allocator for each unknown sequence.
    ///
    /// Only numbers the parser does not implement are reported this way.
    /// A sequence with a number the parser does implement stays
    /// [`CommandType::Invalid`] when its contents are malformed.
    ///
    /// Options stay set across [`Parser::reset()`]. You can change an option
    /// at any time, but a sequence that is already being parsed may keep the
    /// old setting. It is simplest to set options before the first sequence.
    ///
    /// # Example
    ///
    /// ```rust
    /// use libghostty_vt::osc;
    ///
    /// let mut parser = osc::Parser::new().unwrap();
    ///
    /// // Keep up to 1 KiB of each unknown sequence.
    /// parser.set_unknown_max_bytes(1024);
    ///
    /// // Feed the bytes between ESC ] and the terminator. OSC 7400 is made up
    /// // for this example, so the parser does not implement it.
    /// for &b in b"7400;status=busy" {
    ///     parser.next_byte(b);
    /// }
    ///
    /// // This sequence ended with BEL (0x07).
    /// let command = parser.end(0x07);
    ///
    /// if let osc::CommandType::Unknown { content, .. } = command.command_type() {
    ///     // content now holds "7400;status=busy".
    ///     assert_eq!(content, b"7400;status=busy");
    /// }
    /// ```
    pub fn set_unknown_max_bytes(&mut self, max_bytes: usize) -> Result<&mut Self> {
        crate::sys::GATE.freeze();

        // Zero is also what the NULL "restore the default" value sets, so
        // always passing a pointer covers both.
        let result = unsafe {
            ffi::ghostty_osc_set(
                self.0.as_raw(),
                ffi::OscOption::UNKNOWN_MAX_BYTES,
                std::ptr::from_ref(&max_bytes).cast(),
            )
        };
        from_result(result)?;
        Ok(self)
    }

    /// Parse the next byte in an OSC sequence.
    ///
    /// Processes a single byte as part of an OSC sequence. The parser maintains
    /// internal state to track the progress through the sequence. Call this
    /// function for each byte in the sequence data.
    ///
    /// When finished pumping the parser with bytes, call [`Parser::end`] to
    /// get the final result.
    pub fn next_byte(&mut self, byte: u8) {
        crate::sys::GATE.freeze();
        unsafe { ffi::ghostty_osc_next(self.0.as_raw(), byte) }
    }

    /// Finalize OSC parsing and retrieve the parsed command.
    ///
    /// Call this after feeding every byte of the sequence to
    /// [`Parser::next_byte`], except the byte that ended it. Pass that byte
    /// here as the terminator.
    ///
    /// If the sequence is not a valid command, the command has type
    /// [`CommandType::Invalid`].
    ///
    /// Commands that reply to the program, such as color queries, end their
    /// reply the same way the request ended. A terminator of 0x07 (BEL) gets a
    /// BEL reply, and any other byte gets an ST reply. Commands that don't
    /// reply ignore the terminator.
    ///
    /// If the program cancelled the sequence with CAN (0x18) or SUB (0x1A),
    /// pass that byte as the terminator. The sequence is then discarded and
    /// the command has type [`CommandType::Invalid`], whatever command it
    /// contained. This matches xterm.
    ///
    /// ```rust
    /// use libghostty_vt::osc::{CommandType, Parser};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut parser = Parser::new()?;
    ///
    /// // The program sent "ESC ] 2 ; hello" to set the window title, then
    /// // sent CAN instead of a terminator.
    /// for byte in *b"2;hello" {
    ///     parser.next_byte(byte);
    /// }
    /// let command = parser.end(0x18);
    /// // So the window title does not change.
    /// assert!(matches!(command.command_type(), CommandType::Invalid));
    /// # Ok(())
    /// # }
    /// ```
    pub fn end(&mut self, terminator: u8) -> Command<'_, 'alloc> {
        crate::sys::GATE.freeze();
        Command {
            // NULL for an invalid or cancelled sequence.
            // `ghostty_osc_command_type` reports that as an invalid command,
            // so data is only ever read from a command of a matched type.
            inner: unsafe { ffi::ghostty_osc_end(self.0.as_raw(), terminator) },
            _parser: PhantomData,
        }
    }
}

impl Drop for Parser<'_> {
    fn drop(&mut self) {
        crate::sys::GATE.freeze();
        unsafe { ffi::ghostty_osc_free(self.0.as_raw()) }
    }
}

/// Opaque handle to a single OSC command.
///
/// This handle represents a parsed OSC (Operating System Command) command.
/// The command can be queried for its type and associated data.
#[derive(Debug)]
pub struct Command<'p, 'alloc> {
    inner: ffi::OscCommand,
    _parser: PhantomData<&'p Parser<'alloc>>,
}

impl<'p> Command<'p, '_> {
    /// Get the type of an OSC command.
    ///
    /// This can be used to determine what kind of command was parsed and
    /// what data might be available from it.
    #[must_use]
    pub fn command_type(self) -> CommandType<'p> {
        self.command_type_inner().unwrap_or(CommandType::Invalid)
    }

    fn command_type_inner(&self) -> Option<CommandType<'p>> {
        use ffi::OscCommandData as Data;
        use ffi::OscCommandType as Type;

        crate::sys::GATE.freeze();
        let raw_type = unsafe { ffi::ghostty_osc_command_type(self.inner) };
        Some(match raw_type {
            Type::CHANGE_WINDOW_TITLE => {
                let title = self.get::<ffi::String>(Data::CHANGE_WINDOW_TITLE_STR)?;
                // SAFETY: The header guarantees the bytes live until the
                // next parser call, which the `'p` borrow prevents.
                let title = unsafe { title.to_bytes() };
                // Upstream doesn't promise UTF-8, so hand the bytes over as
                // they are and let callers decide whether to validate.
                CommandType::ChangeWindowTitle { title }
            }
            Type::CHANGE_WINDOW_ICON => CommandType::ChangeWindowIcon,
            Type::SEMANTIC_PROMPT => CommandType::SemanticPrompt,
            Type::CLIPBOARD_CONTENTS => CommandType::ClipboardContents,
            Type::REPORT_PWD => CommandType::ReportPwd,
            Type::MOUSE_SHAPE => CommandType::MouseShape,
            Type::COLOR_OPERATION => CommandType::ColorOperation,
            Type::KITTY_COLOR_PROTOCOL => CommandType::KittyColorProtocol,
            Type::SHOW_DESKTOP_NOTIFICATION => CommandType::ShowDesktopNotification,
            Type::HYPERLINK_START => CommandType::HyperlinkStart,
            Type::HYPERLINK_END => CommandType::HyperlinkEnd,
            Type::CONEMU_SLEEP => CommandType::ConemuSleep,
            Type::CONEMU_SHOW_MESSAGE_BOX => CommandType::ConemuShowMessageBox,
            Type::CONEMU_CHANGE_TAB_TITLE => CommandType::ConemuChangeTabTitle,
            Type::CONEMU_PROGRESS_REPORT => CommandType::ConemuProgressReport,
            Type::CONEMU_WAIT_INPUT => CommandType::ConemuWaitInput,
            Type::CONEMU_GUIMACRO => CommandType::ConemuGuiMacro,
            Type::CONEMU_RUN_PROCESS => CommandType::ConemuRunProcess,
            Type::CONEMU_OUTPUT_ENVIRONMENT_VARIABLE => {
                CommandType::ConemuOutputEnvironmentVariable
            }
            Type::CONEMU_XTERM_EMULATION => CommandType::ConemuXtermEmulation,
            Type::CONEMU_COMMENT => CommandType::ConemuComment,
            Type::KITTY_TEXT_SIZING => CommandType::KittyTextSizing,
            Type::KITTY_CLIPBOARD_PROTOCOL => CommandType::KittyClipboardProtocol,
            Type::KITTY_DND_PROTOCOL => CommandType::KittyDndProtocol,
            Type::HIERARCHICAL_CONTEXT => CommandType::HierarchicalContext,
            Type::KITTY_DESKTOP_NOTIFICATION => CommandType::KittyDesktopNotification,
            Type::UNKNOWN => {
                // The content string is only valid until the next call to any
                // `ghostty_osc_*` function on this parser, so read the other
                // fields first and the content last.
                let truncated = self.get(Data::UNKNOWN_TRUNCATED)?;
                // A terminator these bindings don't know has no Rust value, so
                // the whole command is reported as invalid rather than
                // guessing one.
                let terminator = self
                    .get::<ffi::OscTerminator::Type>(Data::UNKNOWN_TERMINATOR)?
                    .try_into()
                    .ok()?;
                // SAFETY: We trust libghostty to give us a valid string. It is
                // owned by the parser and valid until the next `ghostty_osc_*`
                // call on it. This is the last such call here, and from now on
                // the `'p` borrow of the parser rules out any further ones.
                let content = unsafe { self.get::<ffi::String>(Data::UNKNOWN_CONTENT)?.to_bytes() };
                CommandType::Unknown {
                    content,
                    truncated,
                    terminator,
                }
            }
            Type::PROGRAM_STATUS => CommandType::ProgramStatus,

            _ => return None,
        })
    }

    fn get<T>(&self, tag: ffi::OscCommandData::Type) -> Option<T> {
        crate::sys::GATE.freeze();

        let mut value = MaybeUninit::<T>::zeroed();
        let result =
            unsafe { ffi::ghostty_osc_command_data(self.inner, tag, value.as_mut_ptr().cast()) };

        if result == ffi::Result::SUCCESS {
            // SAFETY: Value should be initialized after successful call.
            Some(unsafe { value.assume_init() })
        } else {
            None
        }
    }
}

/// Type of an OSC command.
///
/// ```rust
/// use libghostty_vt::osc::{CommandType, Parser};
///
/// // Feed the bytes between `ESC ]` and the terminator, here ST (`ESC \`).
/// fn parse<'p>(parser: &'p mut Parser<'_>, body: &[u8]) -> CommandType<'p> {
///     parser.reset();
///     for &byte in body {
///         parser.next_byte(byte);
///     }
///     parser.end(0x5C).command_type()
/// }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut parser = Parser::new()?;
///
/// // Kitty clipboard protocol (OSC 5522).
/// let command = parse(&mut parser, b"5522;type=read;dGV4dC9wbGFpbg==");
/// assert!(matches!(command, CommandType::KittyClipboardProtocol));
/// // Kitty drag and drop protocol (OSC 72).
/// let command = parse(&mut parser, b"72;t=a:i=5;text/plain text/uri-list");
/// assert!(matches!(command, CommandType::KittyDndProtocol));
/// // Hierarchical context signalling (OSC 3008).
/// let command = parse(&mut parser, b"3008;start=abc123");
/// assert!(matches!(command, CommandType::HierarchicalContext));
/// // Kitty desktop notifications (OSC 99).
/// let command = parse(&mut parser, b"99;i=bobr;payload");
/// assert!(matches!(command, CommandType::KittyDesktopNotification));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
#[expect(missing_docs, reason = "missing upstream docs")]
pub enum CommandType<'p> {
    #[default]
    Invalid,
    /// A window title change (OSC 0 or OSC 2).
    ///
    /// ```rust
    /// use libghostty_vt::osc::{CommandType, Parser};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut parser = Parser::new()?;
    ///
    /// for byte in *b"2;hello" {
    ///     parser.next_byte(byte);
    /// }
    /// let command = parser.end(0x5C);
    /// assert!(matches!(
    ///     command.command_type(),
    ///     CommandType::ChangeWindowTitle { title } if title == b"hello"
    /// ));
    ///
    /// // The title isn't checked for UTF-8: these bytes are passed through.
    /// parser.reset();
    /// for byte in *b"2;\xff" {
    ///     parser.next_byte(byte);
    /// }
    /// let command = parser.end(0x5C);
    /// assert!(matches!(
    ///     command.command_type(),
    ///     CommandType::ChangeWindowTitle { title } if title == b"\xff"
    /// ));
    /// # Ok(())
    /// # }
    /// ```
    ChangeWindowTitle {
        /// Window title string data.
        ///
        /// Memory is owned by the parser and valid until the next call to the
        /// same parser.
        ///
        /// The title is not guaranteed to be UTF-8 or NUL-terminated.
        title: &'p [u8],
    },
    ChangeWindowIcon,
    SemanticPrompt,
    ClipboardContents,
    ReportPwd,
    MouseShape,
    ColorOperation,
    KittyColorProtocol,
    ShowDesktopNotification,
    HyperlinkStart,
    HyperlinkEnd,
    ConemuSleep,
    ConemuShowMessageBox,
    ConemuChangeTabTitle,
    ConemuProgressReport,
    ConemuWaitInput,
    ConemuGuiMacro,
    ConemuRunProcess,
    ConemuOutputEnvironmentVariable,
    ConemuXtermEmulation,
    ConemuComment,
    KittyTextSizing,
    KittyClipboardProtocol,
    KittyDndProtocol,
    HierarchicalContext,
    KittyDesktopNotification,

    /// An OSC sequence whose number the parser does not implement.
    ///
    /// Only produced when the limit set with [`Parser::set_unknown_max_bytes`] is nonzero.
    /// Otherwise these sequences are [`CommandType::Invalid`].
    Unknown {
        /// The raw bytes of an unknown sequence: everything that was passed to
        /// [`Parser::next_byte`], including the number at the start. For
        /// example, the sequence `ESC ] 7400;status=busy BEL` gives
        /// `7400;status=busy`.
        ///
        /// Owned by the parser and valid until the next call to the same
        /// parser.
        content: &'p [u8],
        /// True if the unknown sequence was longer than the limit set with
        /// [`Parser::set_unknown_max_bytes`], or memory ran out while reading
        /// it. In that case the content holds only the beginning of the
        /// sequence.
        truncated: bool,
        /// How the unknown sequence was ended, based on the terminator passed
        /// to [`Parser::end`]. If you reply to the sequence, end the reply the
        /// same way. A terminator from a newer libghostty that these bindings
        /// don't know yet gives [`CommandType::Invalid`] instead (see
        /// [`Terminator`]).
        terminator: Terminator,
    },

    /// A program status report or support query (OSC 7501), which a program
    /// sends to say what it is doing, such as working or waiting on the user.
    ///
    /// The OSC parser only identifies this command. To receive the report's
    /// contents, use a terminal with [`Terminal::on_program_status`] instead
    /// (see [`ProgramStatus`]).
    ///
    /// [`Terminal::on_program_status`]: crate::Terminal::on_program_status
    /// [`ProgramStatus`]: crate::effect::ProgramStatus
    ProgramStatus,
}

/// How an OSC sequence was ended.
///
/// Programs can end an OSC sequence in two ways. When you reply to a
/// sequence, end the reply the same way the program ended its request.
/// Some programs only recognize replies that match.
///
/// Newer versions of libghostty may add terminators. A terminator these
/// bindings don't know yet makes [`Command::command_type`] give
/// [`CommandType::Invalid`], and makes
/// [`Terminal::on_unknown_sequence`](crate::Terminal::on_unknown_sequence)
/// skip the sequence.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum Terminator {
    /// The string terminator (ST): ESC followed by a backslash (0x1B 0x5C).
    St = ffi::OscTerminator::ST,
    /// The bell character, BEL (byte 0x07).
    Bel = ffi::OscTerminator::BEL,
}
