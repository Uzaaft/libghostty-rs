//! Encoding mouse events into terminal escape sequences.
//!
//! Supports X10, UTF-8, SGR, URxvt, and SGR-Pixels mouse protocols.
//!
//! # Basic Usage
//!
//!  1. Create an encoder instance with [`Encoder::new`].
//!  2. Configure encoder options with the various `Encoder::with_*` methods
//!     or [`Encoder::with_options_from_terminal`].
//!  3. For each mouse event:
//!     *  Create a mouse event with [`Event::new`] (or reuse an old one).
//!     *  Set event properties (action, button, modifiers, position).
//!     *  Encode with [`Encoder::encode`].

use crate::{
    alloc::{Allocator, Object},
    error::{Result, from_result, from_result_with_len},
    ffi, key,
    terminal::Terminal,
};

#[doc(inline)]
pub use ffi::GhosttyMousePosition as Position;

/// Mouse encoder that converts normalized mouse events into
/// terminal escape sequences.
pub struct Encoder<'alloc>(Object<'alloc, ffi::GhosttyMouseEncoder>);

impl<'alloc> Encoder<'alloc> {
    /// Create a new mouse encoder instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new mouse encoder instance with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(alloc: &'alloc Allocator<'ctx, Ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator) -> Result<Self> {
        let mut raw: ffi::GhosttyMouseEncoder_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_mouse_encoder_new(alloc, &mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    unsafe fn setopt(
        &mut self,
        option: ffi::GhosttyMouseEncoderOption,
        value: *const std::ffi::c_void,
    ) {
        unsafe { ffi::ghostty_mouse_encoder_setopt(self.0.as_raw(), option, value) }
    }

    /// Encode a mouse event into a terminal escape sequence.
    ///
    /// Not all mouse events produce output. In such cases this returns `Ok(0)`.
    ///
    /// If the output buffer is too small, this returns
    /// `Err(Error::OutOfSpace { required })` where `required` is the required size.
    pub fn encode(&mut self, event: &Event, buf: &mut [u8]) -> Result<usize> {
        let mut written: usize = 0;
        let result = unsafe {
            ffi::ghostty_mouse_encoder_encode(
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
    /// This sets tracking mode and output format from terminal state.
    /// It does not modify size or any-button state.
    pub fn with_options_from_terminal(&mut self, terminal: &Terminal<'_, '_>) -> &mut Self {
        unsafe {
            ffi::ghostty_mouse_encoder_setopt_from_terminal(
                self.0.as_raw(),
                terminal.inner.as_raw(),
            )
        }
        self
    }
    /// Set mouse tracking mode.
    pub fn with_tracking_mode(&mut self, value: TrackingMode) -> &mut Self {
        unsafe {
            self.setopt(
                ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_EVENT,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set mouse output format.
    pub fn with_format(&mut self, value: Format) -> &mut Self {
        unsafe {
            self.setopt(
                ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_EVENT,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set renderer size context.
    pub fn with_size(&mut self, value: EncoderSize) -> &mut Self {
        let raw: ffi::GhosttyMouseEncoderSize = value.into();
        unsafe {
            self.setopt(
                ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
                std::ptr::from_ref(&raw).cast(),
            )
        }
        self
    }
    /// Set whether any mouse button is currently pressed.
    pub fn with_any_button_pressed(&mut self, value: bool) -> &mut Self {
        unsafe {
            self.setopt(
                ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }
    /// Set whether to enable motion deduplication by last cell.
    pub fn with_track_last_cell(&mut self, value: bool) -> &mut Self {
        unsafe {
            self.setopt(
                ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL,
                std::ptr::from_ref(&value).cast(),
            )
        }
        self
    }

    /// Reset internal encoder state.
    ///
    /// This clears motion deduplication state (last tracked cell).
    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_mouse_encoder_reset(self.0.as_raw()) }
    }
}

impl Drop for Encoder<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_mouse_encoder_free(self.0.as_raw()) }
    }
}

/// Normalized mouse input event containing action, button, modifiers, and
/// surface-space position.
pub struct Event<'alloc>(Object<'alloc, ffi::GhosttyMouseEvent>);

impl<'alloc> Event<'alloc> {
    /// Create a new mouse event instance.
    pub fn new() -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null()) }
    }

    /// Create a new mouse event instance with a custom allocator.
    ///
    /// See the [crate-level documentation](crate#memory-management-and-lifetimes)
    /// regarding custom memory management and lifetimes.
    pub fn new_with_alloc<'ctx: 'alloc, Ctx>(alloc: &'alloc Allocator<'ctx, Ctx>) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw()) }
    }

    unsafe fn new_inner(alloc: *const ffi::GhosttyAllocator) -> Result<Self> {
        let mut raw: ffi::GhosttyMouseEvent_ptr = std::ptr::null_mut();
        let result = unsafe { ffi::ghostty_mouse_event_new(alloc, &mut raw) };
        from_result(result)?;
        Ok(Self(Object::new(raw)?))
    }

    /// Set the event action.
    pub fn set_action(&mut self, action: Action) {
        unsafe {
            ffi::ghostty_mouse_event_set_action(self.0.as_raw(), action as ffi::GhosttyMouseAction)
        }
    }

    /// Get the event action.
    pub fn action(&self) -> Action {
        unsafe { ffi::ghostty_mouse_event_get_action(self.0.as_raw()) }
            .try_into()
            .unwrap_or(Action::Press)
    }

    /// Set the event button.
    pub fn set_button(&mut self, button: Option<Button>) {
        if let Some(button) = button {
            unsafe {
                ffi::ghostty_mouse_event_set_button(
                    self.0.as_raw(),
                    button as ffi::GhosttyMouseButton,
                )
            }
        } else {
            unsafe { ffi::ghostty_mouse_event_clear_button(self.0.as_raw()) }
        }
    }

    /// Get the event button.
    pub fn button(&self) -> Option<Button> {
        let mut button: ffi::GhosttyMouseButton = 0;
        let has_button =
            unsafe { ffi::ghostty_mouse_event_get_button(self.0.as_raw(), &mut button) };
        if has_button {
            Some(button.try_into().unwrap_or(Button::Unknown))
        } else {
            None
        }
    }

    /// Set keyboard modifiers held during the event.
    pub fn set_mods(&mut self, mods: key::Mods) {
        unsafe { ffi::ghostty_mouse_event_set_mods(self.0.as_raw(), mods.bits()) }
    }

    /// Get keyboard modifiers held during the event.
    pub fn mods(&self) -> key::Mods {
        key::Mods::from_bits_retain(
            unsafe { ffi::ghostty_mouse_event_get_mods(self.0.as_raw()) }.into(),
        )
    }

    /// Set the event position in surface-space pixels.
    pub fn set_position(&mut self, pos: Position) {
        unsafe { ffi::ghostty_mouse_event_set_position(self.0.as_raw(), pos) }
    }

    /// Get the event position in surface-space pixels.
    pub fn position(&self) -> Position {
        unsafe { ffi::ghostty_mouse_event_get_position(self.0.as_raw()) }
    }
}

impl Drop for Event<'_> {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_mouse_event_free(self.0.as_raw()) }
    }
}

/// Mouse tracking mode.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum TrackingMode {
    /// Mouse reporting disabled.
    None = ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_NONE,
    /// X10 mouse mode.
    X10 = ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_X10,
    /// Normal mouse mode (press/release only).
    Normal = ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_NORMAL,
    /// Button-event tracking mode.
    Button = ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_BUTTON,
    /// Any-event tracking mode.
    Any = ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_ANY,
}

/// Mouse output format.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum Format {
    X10 = ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_X10,
    Utf8 = ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_UTF8,
    Sgr = ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_SGR,
    Urxvt = ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_URXVT,
    SgrPixels = ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_SGR_PIXELS,
}

/// Mouse encoder size and geometry context.
///
/// This describes the rendered terminal geometry used to convert surface-space
/// positions into encoded coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderSize {
    /// Full screen width in pixels.
    pub screen_width: u32,
    /// Full screen height in pixels.
    pub screen_height: u32,
    /// Cell width in pixels. Must be non-zero.
    pub cell_width: u32,
    /// Cell height in pixels. Must be non-zero.
    pub cell_height: u32,
    /// Top padding in pixels.
    pub padding_top: u32,
    /// Bottom padding in pixels.
    pub padding_bottom: u32,
    /// Right padding in pixels.
    pub padding_right: u32,
    /// Left padding in pixels.
    pub padding_left: u32,
}

impl From<EncoderSize> for ffi::GhosttyMouseEncoderSize {
    fn from(value: EncoderSize) -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            screen_width: value.screen_width,
            screen_height: value.screen_height,
            cell_width: value.cell_width,
            cell_height: value.cell_height,
            padding_top: value.padding_top,
            padding_bottom: value.padding_bottom,
            padding_right: value.padding_right,
            padding_left: value.padding_left,
        }
    }
}

/// Mouse event action type.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum Action {
    /// Mouse button was pressed.
    Press = ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_PRESS,
    /// Mouse button was released.
    Release = ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_RELEASE,
    /// Mouse moved.
    Motion = ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_MOTION,
}

/// Mouse event action identity.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, int_enum::IntEnum)]
#[non_exhaustive]
pub enum Button {
    Unknown = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_UNKNOWN,
    Left = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_LEFT,
    Right = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_RIGHT,
    Middle = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_MIDDLE,
    Four = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FOUR,
    Five = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FIVE,
    Six = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_SIX,
    Seven = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_SEVEN,
    Eight = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_EIGHT,
    Nine = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_NINE,
    Ten = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_TEN,
    Eleven = ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_ELEVEN,
}
