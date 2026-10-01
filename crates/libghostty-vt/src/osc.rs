//! Handling OSC (Operating System Command) escape sequences.

use std::{
    ffi::{CStr, c_char},
    marker::PhantomData,
    mem::MaybeUninit,
};

use crate::{
    alloc::{Allocator, Object},
    error::{Result, from_result},
    ffi,
};

/// OSC (Operating System Command) sequence parser and command handling.
///
/// The parser operates in a streaming fashion, processing input byte-by-byte
/// to handle OSC sequences that may arrive in fragments across multiple reads.
/// This interface makes it easy to integrate into most environments and avoids
/// over-allocating buffers.
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
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc>(alloc: &'alloc Allocator<'ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::Allocator) -> Result<Self> {
        let mut raw: ffi::OscParser = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_osc_new(alloc, &raw mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    /// Set the most bytes to keep from each OSC sequence whose number the
    /// parser does not implement.
    ///
    /// Zero, the default, discards these sequences and they produce
    /// [`CommandType::Invalid`]. Any other value makes them produce
    /// [`CommandType::Unknown`].
    ///
    /// A sequence longer than the limit is still reported. Its content holds
    /// the first bytes up to the limit, and `truncated` is true.
    ///
    /// Limits up to 2048 bytes use a buffer the parser already owns and never
    /// allocate memory. Larger limits allocate memory from the parser's
    /// allocator for each unknown sequence.
    ///
    /// The limit stays set across [`Self::reset`]. You can change it at any
    /// time, but a sequence that is already being parsed may keep the old
    /// setting. It is simplest to set it before the first sequence.
    pub fn set_unknown_max_bytes(&mut self, max: usize) -> Result<&mut Self> {
        // SAFETY: `max` is the `size_t` the option expects, and libghostty
        // only reads it during the call.
        let result = unsafe {
            ffi::ghostty_osc_set(
                self.0.as_raw(),
                ffi::OscOption::UNKNOWN_MAX_BYTES,
                std::ptr::from_ref(&max).cast(),
            )
        };
        from_result(result)?;
        Ok(self)
    }

    /// Reset an OSC parser instance to its initial state.
    ///
    /// Resets the parser state, clearing any partially parsed OSC sequences
    /// and returning the parser to its initial state. This is useful for
    /// reusing a parser instance or recovering from parse errors.
    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_osc_reset(self.0.as_raw()) }
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
        unsafe { ffi::ghostty_osc_next(self.0.as_raw(), byte) }
    }

    /// Finalize OSC parsing and retrieve the parsed command.
    ///
    /// Call this after feeding every byte of the sequence to
    /// [`Parser::next_byte`], except the byte that ended it. Pass that byte
    /// here as the terminator: 0x07 for BEL, 0x5C for ST, or 0x18 (CAN) or
    /// 0x1A (SUB) if it was cancelled.
    /// Call [`Parser::reset`] before parsing the next sequence.
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
    pub fn end<'p>(&'p mut self, terminator: u8) -> Command<'p, 'alloc> {
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
        unsafe { ffi::ghostty_osc_free(self.0.as_raw()) }
    }
}

/// A parsed OSC (Operating System Command) command.
///
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

        let raw_type = unsafe { ffi::ghostty_osc_command_type(self.inner) };
        Some(match raw_type {
            Type::CHANGE_WINDOW_TITLE => {
                // The data is a pointer to a NUL-terminated string, not a
                // Rust string slice.
                let title = self.get::<*const c_char>(Data::CHANGE_WINDOW_TITLE_STR)?;
                CommandType::ChangeWindowTitle {
                    // SAFETY: A successful query never yields NULL. The string
                    // is owned by the parser and valid until the next call on
                    // it, which the `'p` borrow of the parser rules out.
                    title: unsafe { CStr::from_ptr(title) },
                }
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
            Type::CONTEXT_SIGNAL => CommandType::ContextSignal,
            Type::KITTY_DESKTOP_NOTIFICATION => CommandType::KittyDesktopNotification,
            Type::UNKNOWN => {
                let content = self.get::<ffi::String>(Data::UNKNOWN_CONTENT)?;
                CommandType::Unknown {
                    // SAFETY: The bytes are owned by the parser and valid until
                    // the next call on it, which the `'p` borrow of the parser
                    // rules out.
                    content: unsafe { content.to_bytes() },
                    truncated: self.get(Data::UNKNOWN_TRUNCATED)?,
                    // Upstream answers anything but BEL with ST, so a
                    // terminator added later is best answered the same way.
                    terminator: self
                        .get::<ffi::OscTerminator::Type>(Data::UNKNOWN_TERMINATOR)?
                        .try_into()
                        .unwrap_or(Terminator::St),
                }
            }

            _ => return None,
        })
    }

    fn get<T>(&self, tag: ffi::OscCommandData::Type) -> Option<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let result =
            unsafe { ffi::ghostty_osc_command_data(self.inner, tag, value.as_mut_ptr().cast()) };

        if result {
            // SAFETY: Value should be initialized after successful call.
            Some(unsafe { value.assume_init() })
        } else {
            None
        }
    }
}

/// Type of an OSC command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
#[expect(missing_docs, reason = "missing upstream docs")]
pub enum CommandType<'p> {
    #[default]
    Invalid,
    ChangeWindowTitle {
        /// Window title string data.
        ///
        /// The title comes straight from the running program, so it is not
        /// guaranteed to be valid UTF-8. It is valid until the next call on
        /// the [`Parser`] it came from.
        title: &'p CStr,
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
    ContextSignal,
    KittyDesktopNotification,
    /// An OSC sequence whose number the parser does not implement.
    ///
    /// Only produced when [`Parser::set_unknown_max_bytes`] is nonzero.
    /// Otherwise these sequences are [`CommandType::Invalid`].
    Unknown {
        /// The raw bytes of the sequence: everything that was passed to
        /// [`Parser::next_byte`], including the number at the start. For
        /// example, the sequence `ESC ] 7400;status=busy BEL` gives
        /// `7400;status=busy`. The bytes are valid until the next call on the
        /// [`Parser`] they came from.
        content: &'p [u8],
        /// True if the sequence was longer than
        /// [`Parser::set_unknown_max_bytes`], or memory ran out while reading
        /// it. In that case the content holds only the beginning of the
        /// sequence.
        truncated: bool,
        /// How the sequence was ended, based on the terminator passed to
        /// [`Parser::end`]. If you reply to the sequence, end the reply the
        /// same way.
        terminator: Terminator,
    },
}

/// How an OSC sequence was ended.
///
/// Programs can end an OSC sequence in two ways. When you reply to a
/// sequence, end the reply the same way the program ended its request.
/// Some programs only recognize replies that match.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum Terminator {
    /// The string terminator (ST): ESC followed by a backslash (0x1B 0x5C).
    St = ffi::OscTerminator::ST,
    /// The bell character, BEL (byte 0x07).
    Bel = ffi::OscTerminator::BEL,
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;

    /// Parse the contents of one OSC sequence terminated by BEL.
    fn parse(parser: &mut Parser<'_>, osc: &[u8]) -> String {
        parser.reset();
        for &byte in osc {
            parser.next_byte(byte);
        }
        format!("{:?}", parser.end(0x07).command_type())
    }

    #[test]
    fn unknown_commands_are_reported_once_enabled() {
        let mut parser = Parser::new().unwrap();
        let end = |parser: &mut Parser<'_>, osc: &[u8], terminator: u8| {
            parser.reset();
            for &byte in osc {
                parser.next_byte(byte);
            }
            match parser.end(terminator).command_type() {
                CommandType::Unknown {
                    content,
                    truncated,
                    terminator,
                } => Some((content.to_vec(), truncated, terminator)),
                CommandType::Invalid => None,
                other => panic!("expected an unknown or invalid command, got {other:?}"),
            }
        };

        // By default, an OSC number the parser doesn't implement is invalid.
        assert_eq!(end(&mut parser, b"7400;status=busy", 0x07), None);

        parser.set_unknown_max_bytes(1024).unwrap();
        assert_eq!(
            end(&mut parser, b"7400;status=busy", 0x07),
            Some((b"7400;status=busy".to_vec(), false, Terminator::Bel))
        );
        // The ST terminator is the backslash after ESC.
        assert_eq!(
            end(&mut parser, b"7400;status=busy", 0x5c),
            Some((b"7400;status=busy".to_vec(), false, Terminator::St))
        );
        // The limit stays set across resets, and longer sequences are cut
        // short.
        parser.set_unknown_max_bytes(4).unwrap();
        assert_eq!(
            end(&mut parser, b"7400;status=busy", 0x07),
            Some((b"7400".to_vec(), true, Terminator::Bel))
        );
        // Numbers the parser implements are never unknown.
        assert_eq!(
            parse(&mut parser, b"2;hello"),
            r#"ChangeWindowTitle { title: "hello" }"#
        );
    }

    #[test]
    fn window_title_is_extracted() {
        let mut parser = Parser::new().unwrap();
        for byte in *b"2;hello" {
            parser.next_byte(byte);
        }
        let CommandType::ChangeWindowTitle { title } = parser.end(0x07).command_type() else {
            panic!("expected a window title command");
        };
        assert_eq!(title, c"hello");

        // OSC 0 sets the title too.
        assert_eq!(
            parse(&mut parser, b"0;other"),
            r#"ChangeWindowTitle { title: "other" }"#
        );
    }

    #[test]
    fn newer_protocol_commands_are_recognized() {
        // Payloads taken from upstream's parser tests.
        let mut parser = Parser::new().unwrap();
        let cases: [(&[u8], &str); 4] = [
            (b"5522;type=read;dGV4dC9wbGFpbg==", "KittyClipboardProtocol"),
            (b"72;t=a:i=5;text/plain text/uri-list", "KittyDndProtocol"),
            (b"3008;start=abc123", "ContextSignal"),
            (b"99;;bobr", "KittyDesktopNotification"),
        ];
        for (osc, expected) in cases {
            assert_eq!(
                parse(&mut parser, osc),
                expected,
                "{}",
                String::from_utf8_lossy(osc)
            );
        }
    }
}
