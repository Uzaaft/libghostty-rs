//! Device Control String command parsing.
//!
//! DCS sequences are parsed as a three-stage stream: the terminal parser first
//! reports the hook metadata, then passes payload bytes, then reports unhook.
//! [`Handler`] keeps only the state required to turn that stream into a typed
//! [`Command`].
//!
//! # Basic Usage
//!
//! ```rust
//! use libghostty_vt::dcs::{Command, Dcs, Handler};
//!
//! let mut handler = Handler::new();
//! assert!(handler.hook(&Dcs::new([], [b'+'], b'q')).is_none());
//!
//! for byte in b"536d756C78" {
//!     assert!(handler.put(*byte).is_none());
//! }
//!
//! let Some(Command::Xtgettcap(mut command)) = handler.unhook() else {
//!     panic!("expected XTGETTCAP command");
//! };
//!
//! assert_eq!(command.next_key(), Some("536D756C78"));
//! assert_eq!(command.next_key(), None);
//! ```

/// DCS hook metadata from the terminal parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dcs {
    /// Numeric DCS parameters.
    pub params: Vec<u16>,
    /// Intermediate bytes between the parameter string and final byte.
    pub intermediates: Vec<u8>,
    /// Final DCS byte.
    pub final_byte: u8,
}

impl Dcs {
    /// Construct DCS hook metadata.
    #[must_use]
    pub fn new(
        params: impl Into<Vec<u16>>,
        intermediates: impl Into<Vec<u8>>,
        final_byte: u8,
    ) -> Self {
        Self {
            params: params.into(),
            intermediates: intermediates.into(),
            final_byte,
        }
    }
}

/// Streaming DCS command handler.
#[derive(Debug)]
pub struct Handler {
    state: State,
    /// Maximum passthrough bytes accepted for one command.
    pub max_bytes: usize,
}

impl Handler {
    /// Construct an inactive handler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Inactive,
            max_bytes: 1024 * 1024,
        }
    }

    /// Start a DCS command.
    ///
    /// Unknown hooks enter ignore mode and return `None`.
    pub fn hook(&mut self, dcs: &Dcs) -> Option<Command> {
        debug_assert!(matches!(self.state, State::Inactive));
        self.state = State::Ignore;

        match (dcs.intermediates.as_slice(), dcs.final_byte) {
            ([], b'p') if dcs.params.as_slice() == [1000] => {
                self.state = State::Tmux;
                Some(Command::Tmux(TmuxNotification::Enter))
            }
            ([b'+'], b'q') => {
                self.state = State::Xtgettcap(Vec::with_capacity(128));
                None
            }
            ([b'$'], b'q') => {
                self.state = State::Decrqss(Vec::new());
                None
            }
            _ => None,
        }
    }

    /// Feed one DCS passthrough byte.
    ///
    /// Overflow discards the active command and leaves the handler ignoring
    /// remaining bytes until unhook.
    pub fn put(&mut self, byte: u8) -> Option<Command> {
        match &mut self.state {
            State::Inactive | State::Ignore | State::Tmux => None,
            State::Xtgettcap(bytes) => {
                if bytes.len() >= self.max_bytes {
                    self.discard_to_ignore();
                    return None;
                }
                bytes.push(byte);
                None
            }
            State::Decrqss(bytes) => {
                if bytes.len() >= 2 {
                    self.discard_to_ignore();
                    return None;
                }
                bytes.push(byte);
                None
            }
        }
    }

    /// Finish the DCS command.
    pub fn unhook(&mut self) -> Option<Command> {
        let state = std::mem::replace(&mut self.state, State::Inactive);
        match state {
            State::Inactive | State::Ignore => None,
            State::Tmux => Some(Command::Tmux(TmuxNotification::Exit)),
            State::Xtgettcap(mut bytes) => {
                bytes.make_ascii_uppercase();
                Some(Command::Xtgettcap(Xtgettcap { bytes, index: 0 }))
            }
            State::Decrqss(bytes) => Some(Command::Decrqss(Decrqss::from_bytes(&bytes))),
        }
    }

    fn discard_to_ignore(&mut self) {
        self.state = State::Ignore;
    }
}

impl Default for Handler {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
enum State {
    Inactive,
    Ignore,
    Xtgettcap(Vec<u8>),
    Decrqss(Vec<u8>),
    Tmux,
}

/// Parsed DCS command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// XTGETTCAP key request.
    Xtgettcap(Xtgettcap),
    /// DECRQSS setting request.
    Decrqss(Decrqss),
    /// Tmux control-mode transition.
    Tmux(TmuxNotification),
}

/// XTGETTCAP key iterator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xtgettcap {
    bytes: Vec<u8>,
    index: usize,
}

impl Xtgettcap {
    /// Return the next requested key without hex-decoding it.
    pub fn next_key(&mut self) -> Option<&str> {
        if self.index >= self.bytes.len() {
            return None;
        }

        let rest = &self.bytes[self.index..];
        let end = rest
            .iter()
            .position(|byte| *byte == b';')
            .unwrap_or(rest.len());
        self.index += end + 1;

        std::str::from_utf8(&rest[..end]).ok()
    }
}

/// Supported DECRQSS setting requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decrqss {
    /// Unknown or unsupported setting.
    None,
    /// SGR attributes.
    Sgr,
    /// Cursor style.
    Decscusr,
    /// Top/bottom margins.
    Decstbm,
    /// Left/right margins.
    Decslrm,
}

impl Decrqss {
    fn from_bytes(bytes: &[u8]) -> Self {
        match bytes {
            [b'm'] => Self::Sgr,
            [b'r'] => Self::Decstbm,
            [b's'] => Self::Decslrm,
            [b' ', b'q'] => Self::Decscusr,
            _ => Self::None,
        }
    }
}

/// Tmux control-mode transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TmuxNotification {
    /// Tmux control mode entered.
    Enter,
    /// Tmux control mode exited by DCS unhook.
    Exit,
}

#[cfg(test)]
mod tests {
    use super::{Command, Dcs, Decrqss, Handler, TmuxNotification};

    fn feed(handler: &mut Handler, bytes: &[u8]) {
        for byte in bytes {
            assert_eq!(handler.put(*byte), None);
        }
    }

    #[test]
    fn unknown_dcs_command_is_ignored_without_poisoning_next_command() {
        let mut handler = Handler::new();

        assert_eq!(handler.hook(&Dcs::new([], [], b'A')), None);
        feed(&mut handler, b"ignored");
        assert_eq!(handler.unhook(), None);

        assert_eq!(handler.hook(&Dcs::new([], [b'+'], b'q')), None);
        feed(&mut handler, b"436f");
        let Some(Command::Xtgettcap(mut command)) = handler.unhook() else {
            panic!("expected XTGETTCAP command after ignored command");
        };
        assert_eq!(command.next_key(), Some("436F"));
    }

    #[test]
    fn xtgettcap_command_uppercases_single_key() {
        let mut handler = Handler::new();

        assert_eq!(handler.hook(&Dcs::new([], [b'+'], b'q')), None);
        feed(&mut handler, b"536d756C78");

        let Some(Command::Xtgettcap(mut command)) = handler.unhook() else {
            panic!("expected XTGETTCAP command");
        };
        assert_eq!(command.next_key(), Some("536D756C78"));
        assert_eq!(command.next_key(), None);
    }

    #[test]
    fn xtgettcap_command_iterates_multiple_keys_and_invalid_data() {
        let mut handler = Handler::new();

        assert_eq!(handler.hook(&Dcs::new([], [b'+'], b'q')), None);
        feed(&mut handler, b"who;536D756C78");

        let Some(Command::Xtgettcap(mut command)) = handler.unhook() else {
            panic!("expected XTGETTCAP command");
        };
        assert_eq!(command.next_key(), Some("WHO"));
        assert_eq!(command.next_key(), Some("536D756C78"));
        assert_eq!(command.next_key(), None);
    }

    #[test]
    fn xtgettcap_overflow_discards_command() {
        let mut handler = Handler::new();
        handler.max_bytes = 2;

        assert_eq!(handler.hook(&Dcs::new([], [b'+'], b'q')), None);
        feed(&mut handler, b"abc");
        feed(&mut handler, b"ignored-after-overflow");
        assert_eq!(handler.unhook(), None);

        assert_eq!(handler.hook(&Dcs::new([], [b'$'], b'q')), None);
        feed(&mut handler, b"m");
        assert_eq!(handler.unhook(), Some(Command::Decrqss(Decrqss::Sgr)));
    }

    #[test]
    fn decrqss_command_maps_supported_settings() {
        for (bytes, expected) in [
            (&b"m"[..], Decrqss::Sgr),
            (&b"r"[..], Decrqss::Decstbm),
            (&b"s"[..], Decrqss::Decslrm),
            (&b" q"[..], Decrqss::Decscusr),
            (&b"z"[..], Decrqss::None),
        ] {
            let mut handler = Handler::new();
            assert_eq!(handler.hook(&Dcs::new([], [b'$'], b'q')), None);
            feed(&mut handler, bytes);
            assert_eq!(handler.unhook(), Some(Command::Decrqss(expected)));
        }
    }

    #[test]
    fn decrqss_overlong_command_is_ignored() {
        let mut handler = Handler::new();

        assert_eq!(handler.hook(&Dcs::new([], [b'$'], b'q')), None);
        feed(&mut handler, b" xq");
        assert_eq!(handler.unhook(), None);
    }

    #[test]
    fn tmux_enter_and_implicit_exit() {
        let mut handler = Handler::new();

        assert_eq!(
            handler.hook(&Dcs::new([1000], [], b'p')),
            Some(Command::Tmux(TmuxNotification::Enter)),
        );
        assert_eq!(
            handler.unhook(),
            Some(Command::Tmux(TmuxNotification::Exit)),
        );
    }
}
