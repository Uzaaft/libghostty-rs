//! Handling OSC (Operating System Command) escape sequences.

use std::{marker::PhantomData, mem::MaybeUninit};

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
            Type::CHANGE_WINDOW_TITLE => CommandType::ChangeWindowTitle {
                title: self.get(Data::CHANGE_WINDOW_TITLE_STR)?,
            },
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
#[derive(Debug, Clone, Default)]
#[expect(missing_docs, reason = "missing upstream docs")]
pub enum CommandType<'p> {
    #[default]
    Invalid,
    ChangeWindowTitle {
        /// Window title string data.
        title: &'p str,
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
