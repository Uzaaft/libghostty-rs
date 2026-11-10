# Input {#guide-input}

Input is everything a person does that a program needs to hear about: keys,
mouse events, focus changes and pastes. All of it reaches the program the same
way, as bytes on the pty. These functions produce those bytes.

The catch is that the right bytes keep changing. The up arrow is `ESC [ A` until
a program turns on cursor key application mode, and then it's `ESC O A`. A
program can turn on the Kitty keyboard protocol and get a different encoding for
its keys. A click is nothing at all until a program turns on mouse tracking, and
then it's one of five formats. A paste is framed differently under bracketed
paste.

So an encoder has to follow the terminal's modes, or it writes the wrong bytes.

If you have a [`Terminal`], following its modes takes one call per encoder.
That's the first section, and it's the one thing on this page you have to get
right. If you only feed a terminal output and read the result, you can skip this
page. You need it as soon as a person can type into your terminal.

## Following the terminal's modes {#guide-input-modes}

Each kind of input follows the terminal in its own way:

| Input | How it follows the terminal                                                                     |
| ----- | ----------------------------------------------------------------------------------------------- |
| Keys  | [`key::Encoder::set_options_from_terminal`] copies the modes into the encoder.                      |
| Mouse | [`mouse::Encoder::set_options_from_terminal`] copies the tracking mode and format into the encoder. |
| Focus | There's no encoder. You check the mode yourself.                                                |
| Paste | [`Terminal::paste`] reads the modes itself.                                                |

The two `set_options_from_terminal` calls read the terminal's state when you make them.
They don't stay attached to it, and a program can change its modes in any write.
So sync the encoder after you write to the terminal, or right before you encode.

```rs
// Create a terminal and feed it some VT data that changes modes.
let mut terminal = Terminal::new(80, 24)?;

// The program might enable the Kitty keyboard protocol here.
terminal.vt_write(data);

// Create an encoder and sync its options from the terminal.
let encoder = key::Encoder::new()?;
encoder.set_options_from_terminal(&terminal);

// Encode a key event using the terminal's modes.
let mut buf = [0u8; 128];
let written = encoder.encode(event, &mut buf)?;
```

The mouse works the same way with [`mouse::Encoder::new`],
[`mouse::Encoder::set_options_from_terminal`] and [`mouse::Encoder::encode`].

Each sync leaves some options to you:

- The key sync replaces all of the encoder's options. That covers cursor key
  application mode, keypad mode, ignoring the keypad with numlock, the alt
  escape prefix, backarrow key mode, modifyOtherKeys state and the Kitty
  keyboard flags. It can't know your macOS option key setting, so it resets
  [`key::Encoder::set_macos_option_as_alt`] to [`key::OptionAsAlt::False`].
  If you use that option, set it again after every sync.
- The mouse sync sets the tracking mode and output format. It doesn't touch the
  size or whether a button is held. Those are yours. See [Mouse](#guide-input-mouse).

Without a terminal, there's nothing to follow. Set the options yourself with
the various setters on [`key::Encoder`] and [`mouse::Encoder`], as the examples
below do.

## Keys {#guide-input-keys}

We encode keys with the legacy encoding or the Kitty keyboard protocol,
depending on the encoder's options.

1. Create an encoder with [`key::Encoder::new`].
2. Configure it with [`key::Encoder::set_options_from_terminal`], or with
   other setters if you don't have a terminal.
3. For each key event, create an event with [`key::Event::new`], set its
   action, key, modifiers and so on, and encode it with [`key::Encoder::encode`].
  Then drop it, or change its properties and use it again.

This one has no terminal, so it turns on the Kitty keyboard protocol by hand:

```rs
{{#include ../../../example/rust-vt-encode-key/src/main.rs:key-encode}}
```

Full program: [rust-vt-encode-key/src/main.rs](/example/rust-vt-encode-key/src/main.rs)

Not every key event produces output. An unmodified modifier key usually doesn't,
for example. Those events return `Ok(0)`, so check the count before you write to
the pty.

Most sequences are short, so a fixed buffer of 128 bytes usually does. If it's
too small, you get `Error::OutOfSpace` with the size you need. Discard what's
in the buffer and encode again with a bigger one.

## Mouse {#guide-input-mouse}

We encode mouse events in the X10, UTF-8, SGR, URxvt and SGR-Pixels formats. The
steps are the same as for keys:

1. Create an encoder with [`mouse::Encoder::new`].
2. Configure it with [`mouse::Encoder::set_options_from_terminal`], or with
   other setters if you don't have a terminal.
3. For each mouse event, create an event with [`mouse::Event::new`], set its
   action, button, modifiers and position, and encode it with
   [`mouse::Encoder::encode`]. Then drop it or reuse it.

Event positions are in surface-space pixels, so the encoder has to know your
geometry to turn them into cells. Set it with [`mouse::Encoder::set_size`].
The default is a 1x1 pixel screen with 1x1 pixel cells, which is never what you have.
Call [`mouse::Encoder::set_any_button_pressed`] as well, to say whether a button
is held. Neither one comes from the terminal.

```rs
{{#include ../../../example/rust-vt-encode-mouse/src/main.rs:mouse-encode}}
```

Full program: [rust-vt-encode-mouse/src/main.rs](/example/rust-vt-encode-mouse/src/main.rs)

As with keys, not every event produces output, so check the count. And as with
keys, a 128-byte buffer is enough for typical events.

If you call [`mouse::Encoder::set_track_last_cell`] with `true`, the encoder
deduplicates motion by the last cell. [`mouse::Encoder::reset`] clears that
state.

## Focus {#guide-input-focus}

A program that turns on focus reporting (mode 1004) wants to know when your
terminal gains or loses focus. [`focus::Event::encode`] encodes that as `CSI I`
or `CSI O`. There's no encoder to create and no options to set.

It also doesn't look at the terminal, so it's up to you to check the mode first:

```rs
if terminal.mode(Mode::FOCUS_EVENT).ok_or(false) {
  // Encode the event and write it to the pty.
}
```

If the buffer is too small, you get [`Error::OutOfSpace`] and the size you need.

```rs
{{#include ../../../example/rust-vt-encode-focus/src/main.rs:focus-encode}}
```

Full program: [rust-vt-encode-focus/src/main.rs](/example/rust-vt-encode-focus/src/main.rs)

## Paste {#guide-input-paste}

What a paste writes to the pty depends on the terminal's state, so the way to
paste is [`Terminal::paste`]. You give it the MIME types the clipboard
holds (just `text/plain` for an ordinary paste), a MIME reader callback that
produces the data of any one of them, and where the paste came from. The
terminal decides how its current modes apply.

The bytes come out of the [`Terminal::on_pty_write`] callback, so you need
one installed. See [Effects](effects.md).

### What gets written

The terminal sends the program a paste event instead of the data if all three of
these are true:

- Kitty clipboard protocol paste events (mode 5522, [`Mode::PASTE_EVENTS`])
  are on.
- The person pasted from a clipboard ([`paste::Source::Clipboard`]).
- You've installed a clipboard read callback.

A paste event lists the clipboard's MIME types with a one-time password. No data
is read for it. The program then reads what it wants through your clipboard read
callback, which arrives with `granted` set, so you don't need to ask for
permission.

Otherwise the terminal writes the first text representation. Unsafe control
bytes are replaced with spaces. If mode 2004 ([`Mode::BRACKETED_PASTE`]) is
on, the text is wrapped in bracketed paste sequences. If it's off, newlines are
converted to carriage returns.

We only pull data through the MIME reader when a representation is actually
pasted, so a clipboard holding a large image next to some text costs nothing.
The encoded bytes stream to your callback in chunks as they're produced, never
in one piece. It may run several times for one paste, and you have to write the
pieces to the pty in order.

### Unsafe pastes

Some text could inject commands: a newline or carriage return when the paste
isn't bracketed, or the bracketed paste terminator when it is. We refuse that
text with [`Error::Rejected`] and write nothing, unless GhosttyPaste::allow_unsafe
is set.

The usual flow is to call once, confirm with the person on [`Error::Rejected`],
and call again with `allow_unsafe` set. Each call reads the text at most once
and buffers it whole while the rule is applied. So your source doesn't have to
stay the same between reads. The confirmed retry just pastes whatever the source
holds by then, and a refused or failed paste writes nothing at all.

```rs
{{#include ../../../example/rust-vt-paste/src/main.rs:terminal-paste}}
```

Full program: [rust-vt-paste/src/main.rs](/example/rust-vt-paste/src/main.rs)

### Without a terminal

If you encode without a terminal, there are two building blocks.
[`paste::is_safe`] checks whether paste data contains potentially dangerous
sequences. It's conservative, and it ignores terminal state.

```rs
{{#include ../../../example/rust-vt-paste/src/main.rs:paste-safety}}
```

Full program: [rust-vt-paste/src/main.rs](/example/rust-vt-paste/src/main.rs)

[`paste::encode`] encodes paste data for the pty, including bracketed paste
wrapping and unsafe byte stripping. It modifies your input buffer in place.

```rs
{{#include ../../../example/rust-vt-paste/src/main.rs:paste-encode}}
```

Full program: [rust-vt-paste/src/main.rs](/example/rust-vt-paste/src/main.rs)
