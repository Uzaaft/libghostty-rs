//! Encoding key events into terminal escape sequences,
//!
//! Supports both legacy encoding as well as Kitty Keyboard Protocol.
//!
//! # Basic Usage
//!
//!  1. Create an encoder instance with [`Encoder::new`].
//!  2. Configure encoder options with the various `Encoder::with_*` methods
//!     or [`Encoder::with_options_from_terminal`] if you have a [`Terminal`].
//!  3. For each key event:
//!     *  Create a key event with [`Event::new`] (or reuse an existing one)
//!     *  Set event properties (action, key, modifiers, etc.)
//!     *  Encode with [`Encoder::encode`]
use crate::{
    alloc::{Allocator, Object},
    error::{Result, from_result, from_result_with_len},
    ffi,
    terminal::Terminal,
};

/// Key encoder that converts key events into terminal escape sequences.
pub struct Encoder<'alloc>(Object<'alloc, ffi::GhosttyKeyEncoder>);

impl<'alloc> Encoder<'alloc> {
    /// Create a new key encoder instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new key encoder instance with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(alloc: &'alloc Allocator<'ctx, Ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator) -> Result<Self> {
        let mut raw: ffi::GhosttyKeyEncoder_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_key_encoder_new(alloc, &mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    unsafe fn setopt(
        &mut self,
        option: ffi::GhosttyKeyEncoderOption,
        value: *const std::ffi::c_void,
    ) {
        unsafe { ffi::ghostty_key_encoder_setopt(self.0.as_raw(), option, value) }
    }

    /// Encode a key event into a terminal escape sequence.
    ///
    /// Converts a key event into the appropriate terminal escape sequence
    /// based on the encoder's current options. The sequence is written to
    /// the provided buffer.
    ///
    /// Not all key events produce output. For example, unmodified modifier
    /// keys typically don't generate escape sequences. Check the returned
    /// `usize` to determine if any data was written.
    ///
    /// If the output buffer is too small, this returns
    /// `Err(Error::OutOfSpace { required })` where `required` is the required
    /// buffer size. The caller can then allocate a larger buffer and call
    /// the method again.
    pub fn encode(&mut self, event: &Event, buf: &mut [u8]) -> Result<usize> {
        let mut written: usize = 0;
        let result = unsafe {
            ffi::ghostty_key_encoder_encode(
                self.0.as_raw(),
                event.0.as_raw(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut written,
            )
        };
        from_result_with_len(result, written)
    }

    /// Set encoder options from a terminal's current state.
    ///
    /// Reads the terminal's current modes and flags and applies them to the
    /// encoder's options. This sets cursor key application mode, keypad mode,
    /// alt escape prefix, modifyOtherKeys state, and Kitty keyboard protocol
    /// flags from the terminal state.
    ///
    /// Note that the macos_option_as_alt option cannot be determined from
    /// terminal state and is reset to [`OptionAsAlt::False`] by this call.
    /// Use [`Encoder::with_macos_option_as_alt`] to set it afterward if needed.
    pub fn with_options_from_terminal<UserData>(
        self,
        terminal: &Terminal<'_, '_, UserData>,
    ) -> Self {
        unsafe {
            ffi::ghostty_key_encoder_setopt_from_terminal(self.0.as_raw(), terminal.inner.as_raw())
        }
        self
    }

    /// Set terminal DEC mode 1: cursor key application mode.
    pub fn with_cursor_key_application(mut self, value: bool) -> Self {
        unsafe {
            self.setopt(
                ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_CURSOR_KEY_APPLICATION,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set terminal DEC mode 66: keypad key application mode.
    pub fn with_keypad_key_application(mut self, value: bool) -> Self {
        unsafe {
            self.setopt(
                ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_KEYPAD_KEY_APPLICATION,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set terminal DEC mode 1035: ignore keypad with numlock.
    pub fn with_ignore_keypad_with_numlock(mut self, value: bool) -> Self {
        unsafe {
            self.setopt(
                ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_IGNORE_KEYPAD_WITH_NUMLOCK,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set terminal DEC mode 1036: alt sends escape prefix.
    pub fn with_alt_esc_prefix(mut self, value: bool) -> Self {
        unsafe {
            self.setopt(
                ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_ALT_ESC_PREFIX,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set xterm modifyOtherKeys mode 2.
    pub fn with_modify_other_keys_state_2(mut self, value: bool) -> Self {
        unsafe {
            self.setopt(
                ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_MODIFY_OTHER_KEYS_STATE_2,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set Kitty keyboard protocol flags.
    pub fn with_kitty_flags(mut self, value: KittyKeyFlags) -> Self {
        let value = value.bits();
        unsafe {
            self.setopt(
                ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_KITTY_FLAGS,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set macOS option-as-alt setting.
    pub fn with_macos_option_as_alt(mut self, value: OptionAsAlt) -> Self {
        unsafe {
            self.setopt(
                ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
}

impl Drop for Encoder<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_key_encoder_free(self.0.as_raw()) }
    }
}

/// Keyboard input event containing information about the physical key pressed,
/// modifiers, and generated text.
pub struct Event<'alloc>(Object<'alloc, ffi::GhosttyKeyEvent>);
impl<'alloc> Event<'alloc> {
    /// Create a new key event instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new key event instance with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(alloc: &'alloc Allocator<'ctx, Ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator) -> Result<Self> {
        let mut raw: ffi::GhosttyKeyEvent_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_key_event_new(alloc, &mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    pub fn set_action(&mut self, action: ffi::GhosttyKeyAction) {
        unsafe { ffi::ghostty_key_event_set_action(self.0.as_raw(), action) }
    }

    pub fn get_action(&self) -> ffi::GhosttyKeyAction {
        unsafe { ffi::ghostty_key_event_get_action(self.0.as_raw()) }
    }

    pub fn set_key(&mut self, key: ffi::GhosttyKey) {
        unsafe { ffi::ghostty_key_event_set_key(self.0.as_raw(), key) }
    }

    pub fn get_key(&self) -> ffi::GhosttyKey {
        unsafe { ffi::ghostty_key_event_get_key(self.0.as_raw()) }
    }

    pub fn set_mods(&mut self, mods: ffi::GhosttyMods) {
        unsafe { ffi::ghostty_key_event_set_mods(self.0.as_raw(), mods) }
    }

    pub fn get_mods(&self) -> ffi::GhosttyMods {
        unsafe { ffi::ghostty_key_event_get_mods(self.0.as_raw()) }
    }

    pub fn set_consumed_mods(&mut self, mods: ffi::GhosttyMods) {
        unsafe { ffi::ghostty_key_event_set_consumed_mods(self.0.as_raw(), mods) }
    }

    pub fn get_consumed_mods(&self) -> ffi::GhosttyMods {
        unsafe { ffi::ghostty_key_event_get_consumed_mods(self.0.as_raw()) }
    }

    pub fn set_composing(&mut self, composing: bool) {
        unsafe { ffi::ghostty_key_event_set_composing(self.0.as_raw(), composing) }
    }

    pub fn get_composing(&self) -> bool {
        unsafe { ffi::ghostty_key_event_get_composing(self.0.as_raw()) }
    }

    pub fn set_utf8(&mut self, text: Option<&[u8]>) {
        match text {
            Some(bytes) => unsafe {
                ffi::ghostty_key_event_set_utf8(self.0.as_raw(), bytes.as_ptr().cast(), bytes.len())
            },
            None => unsafe {
                ffi::ghostty_key_event_set_utf8(self.0.as_raw(), std::ptr::null(), 0)
            },
        }
    }

    pub fn set_unshifted_codepoint(&mut self, codepoint: u32) {
        unsafe { ffi::ghostty_key_event_set_unshifted_codepoint(self.0.as_raw(), codepoint) }
    }

    pub fn get_unshifted_codepoint(&self) -> u32 {
        unsafe { ffi::ghostty_key_event_get_unshifted_codepoint(self.0.as_raw()) }
    }
}

impl Drop for Event<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_key_event_free(self.0.as_raw()) }
    }
}

/// macOS option key behavior.
///
/// Determines whether the "option" key on macOS is treated as "alt" or not.
/// See the Ghostty `macos-option-as-alt` configuration option for more details.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
pub enum OptionAsAlt {
    /// Option key is not treated as alt.
    False = ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_FALSE,
    /// Option key is treated as alt.
    True = ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_TRUE,
    /// Only left option key is treated as alt.
    Left = ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_LEFT,
    /// Only right option key is treated as alt.
    Right = ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_RIGHT,
}

bitflags::bitflags! {
    /// Keyboard modifier keys bitmask.
    ///
    /// A bitmask representing all keyboard modifiers. This tracks which modifier
    /// keys are pressed and, where supported by the platform, which side (left or
    /// right) of each modifier is active.
    ///
    /// Modifier side bits are only meaningful when the corresponding modifier bit
    /// is set. Not all platforms support distinguishing between left and right
    /// modifier keys and Ghostty is built to expect that some platforms may not
    /// provide this information.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Mods: u16 {
        /// Shift key is pressed.
        const SHIFT = ffi::GHOSTTY_MODS_SHIFT as u16;
        /// Alt key is pressed.
        const ALT = ffi::GHOSTTY_MODS_ALT as u16;
        /// Control key is pressed.
        const CTRL = ffi::GHOSTTY_MODS_CTRL as u16;
        /// Super/Command/Windows key is pressed.
        const SUPER = ffi::GHOSTTY_MODS_SUPER as u16;
        /// Caps Lock is active.
        const CAPS_LOCK = ffi::GHOSTTY_MODS_CAPS_LOCK as u16;
        /// Num Lock is active.
        const NUM_LOCK = ffi::GHOSTTY_MODS_NUM_LOCK as u16;
        /// Right Shift is pressed (unset = left, set = right).
        ///
        /// Only valid when [`Mods::SHIFT`] is set.
        const SHIFT_SIDE = ffi::GHOSTTY_MODS_SHIFT_SIDE as u16;
        /// Right Alt is pressed (unset = left, set = right).
        ///
        /// Only valid when [`Mods::ALT`] is set.
        const ALT_SIDE = ffi::GHOSTTY_MODS_ALT_SIDE as u16;
        /// Right Control is pressed (unset = left, set = right).
        ///
        /// Only valid when [`Mods::CTRL`] is set.
        const CTRL_SIDE = ffi::GHOSTTY_MODS_CTRL_SIDE as u16;
        /// Right Super is pressed (unset = left, set = right).
        ///
        /// Only valid when [`Mods::SUPER`] is set.
        const SUPER_SIDE = ffi::GHOSTTY_MODS_SUPER_SIDE as u16;
    }

    /// Kitty keyboard protocol flags.
    ///
    /// Bitflags representing the various modes of the Kitty keyboard protocol.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct KittyKeyFlags: u8 {
        /// Kitty keyboard protocol disabled (all flags off).
        const DISABLED = ffi::GHOSTTY_KITTY_KEY_DISABLED as u8;
        /// Disambiguate escape codes.
        const DISAMBIGUATE = ffi::GHOSTTY_KITTY_KEY_DISAMBIGUATE as u8;
        /// Report key press and release events.
        const REPORT_EVENTS = ffi::GHOSTTY_KITTY_KEY_REPORT_EVENTS as u8;
        /// Report alternate key codes.
        const REPORT_ALTERNATES = ffi::GHOSTTY_KITTY_KEY_REPORT_ALTERNATES as u8;
        /// Report all key events including those normally handled by the terminal.
        const REPORT_ALL = ffi::GHOSTTY_KITTY_KEY_REPORT_ALL as u8;
        /// Report associated text with key events
        const REPORT_ASSOCIATED = ffi::GHOSTTY_KITTY_KEY_REPORT_ASSOCIATED as u8;
        /// All Kitty keyboard protocol flags enabled
        const ALL = ffi::GHOSTTY_KITTY_KEY_ALL as u8;
    }
}
