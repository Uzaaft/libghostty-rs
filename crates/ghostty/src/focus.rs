//! Encoding focus gained/lost events into terminal escape sequences
//! (CSI I / CSI O) for focus reporting mode (mode 1004).
//!
//! # Basic Usage
//!
//! Use [`Event::encode`] to encode a focus event into a caller-provided
//! buffer. If the buffer is too small, the method returns
//! `Err(Error::OutOfSpace { required })` where `required` is the required size.
//!
//! # Example
//!
//! ```rust
//! use ghostty::focus::Event;
//! let mut buf = [0u8; 8];
//! if let Ok(written) = Event::Gained.encode(&mut buf) {
//!     println!("Encoded {written} bytes: {:?}", &buf[..written]);
//! }
//! ```

use crate::{
    error::{Result, from_result_with_len},
    ffi,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Gained,
    Lost,
}

impl Event {
    pub fn encode(self, buf: &mut [u8]) -> Result<usize> {
        let mut written: usize = 0;
        let result = unsafe {
            ffi::ghostty_focus_encode(
                self.into(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut written,
            )
        };
        from_result_with_len(result, written)
    }
}

impl From<Event> for ffi::GhosttyFocusEvent {
    fn from(value: Event) -> Self {
        match value {
            Event::Gained => ffi::GhosttyFocusEvent_GHOSTTY_FOCUS_GAINED,
            Event::Lost => ffi::GhosttyFocusEvent_GHOSTTY_FOCUS_LOST,
        }
    }
}
