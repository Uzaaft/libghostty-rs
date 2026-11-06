# Effects {#guide-effects}

An effect is a callback the terminal calls when a program asks for something the
terminal can't do by itself. That means things like ringing the bell, changing
the window title, writing to the clipboard, or answering a query.

Why callbacks? libghostty-vt has no window, no clipboard and no pty. Those are
yours. When a program writes a BEL we know what it means, but we can't make a
sound. So we tell you, and you decide what a bell is.

By default, the VT write functions only process sequences that directly change
terminal state. Anything with a side effect or a reply is ignored until you
register a callback for it. That includes queries, so a program that asks a
question gets no answer.

If you're only feeding bytes into a terminal to read the result back, you can
skip this page. You need effects as soon as a real program runs in the terminal.

## Registering an effect {#guide-effects-registering}

Each effect can be registered by passing a callback into a corresponding function on
[`Terminal`], prefixed with `on_`:

```rs
terminal.on_bell(|_term| println!("Ring-a-ding-a-ding!"));
```

The callbacks always receive a shared reference to the [`Terminal`] as their
first parameter, as well as other parameters depending on the exact callback.

## The rules for callbacks {#guide-effects-rules}

Callbacks run synchronously, in the middle of [`Terminal::vt_write`] or
[`Terminal::vt_write_until`]. All of these rules come from that.

You can't write to the same terminal from a callback. The terminal is still
working through the first write, and it isn't reentrant. This applies to both
[`Terminal::vt_write`] and [`Terminal::vt_write_until`]. For this reason,
callbacks only receive a _shared_ reference to the terminal, which cannot call
mutating functions like [`vt_write`](Terminal::vt_write).

Don't block, and don't do anything expensive. Further IO processing waits until
your callback returns, so a slow callback is a slow terminal.

Copy what you want to keep. Strings and buffers passed to a callback are
borrowed, and they're only valid until it returns.

## The effects {#guide-effects-table}

There are 19 effects:

| Function | Callback type                        | Trigger                                                                 |
| ------------------------------------------ | ------------------------------------ | ----------------------------------------------------------------------- |
| [`on_pty_write`](Terminal::on_pty_write)            | [`PtyWriteFn`](terminal::PtyWriteFn)            | VT query and mode reports written back to the pty                       |
| [`on_bell`](Terminal::on_bell)                 | [`BellFn`](terminal::BellFn)                | BEL character (0x07)                                                    |
| [`on_title_changed`](Terminal::on_title_changed)        | [`TitleChangedFn`](terminal::TitleChangedFn)        | Title change via OSC 0 / OSC 2                                          |
| [`on_pwd_changed`](Terminal::on_pwd_changed)          | [`PwdChangedFn`](terminal::PwdChangedFn)          | Pwd change via OSC 7 / OSC 9 / OSC 1337                                 |
| [`on_enquiry`](Terminal::on_enquiry)              | [`EnquiryFn`](terminal::EnquiryFn)             | ENQ character (0x05)                                                    |
| [`on_xtversion`](Terminal::on_xtversion)            | [`XtversionFn`](terminal::XtversionFn)           | XTVERSION query (`CSI > q`)                                             |
| [`on_size`](Terminal::on_size)                 | [`SizeFn`](terminal::SizeFn)                | XTWINOPS query (`CSI 14/16/18/19 t`) or mode 2048 enable                |
| [`on_color_scheme`](Terminal::on_color_scheme)         | [`ColorSchemeFn`](terminal::ColorSchemeFn)         | Color scheme query (`CSI ? 996 n`)                                      |
| [`on_device_attributes`](Terminal::on_device_attributes)    | [`DeviceAttributesFn`](terminal::DeviceAttributesFn)    | Device attributes query (`CSI c`, `CSI > c`, `CSI = c`)                 |
| [`on_clipboard_write`](Terminal::on_clipboard_write)      | [`ClipboardWriteFn`](terminal::ClipboardWriteFn)      | Clipboard write via OSC 52 / OSC 1337 / OSC 5522                        |
| [`on_clipboard_read`](Terminal::on_clipboard_read)       | [`ClipboardReadFn`](terminal::ClipboardReadFn)       | Clipboard read via OSC 52 `?` / OSC 5522                                |
| [`on_desktop_notification`](Terminal::on_desktop_notification) | [`DesktopNotificationFn`](terminal::DesktopNotificationFn) | Desktop notification via OSC 9 / OSC 777                                |
| [`on_progress_report`](Terminal::on_progress_report)      | [`ProgressReportFn`](terminal::ProgressReportFn)      | Progress report via OSC 9;4                                             |
| [`on_program_status`](Terminal::on_program_status)       | [`ProgramStatusFn`](terminal::ProgramStatusFn)       | Program status report via OSC 7501                                      |
| [`on_unknown_sequence`](Terminal::on_unknown_sequence)     | [`UnknownSequenceFn`](terminal::UnknownSequenceFn)     | APC or OSC sequence we don't implement. See @ref guide-custom-sequences |
| [`on_render_hold`](Terminal::on_render_hold)          | [`RenderHoldFn`](terminal::RenderHoldFn)          | Synchronized output (mode 2026) begins or ends                          |
| [`on_semantic_prompt`](Terminal::on_semantic_prompt)      | [`SemanticPromptFn`](terminal::SemanticPromptFn)      | Shell reports a prompt or command step via OSC 133                      |
| [`on_reset`](Terminal::on_reset)                | [`ResetFn`](terminal::ResetFn)               | Full reset (RIS, `ESC c`)                                               |
| [`on_resize_window`](Terminal::on_resize_window)        | [`ResizeWindowFn`](terminal::ResizeWindowFn)        | Window resize request (`CSI 8 t`)                                       |

## Common callbacks {#guide-effects-common}

These are the callbacks most terminals start with. Each one is from the same
example program, which prints what it receives.

### Replying to the program

Start with this one. Every reply the terminal generates comes through it, such
as the answer to a device status report or a mode query. Write the bytes to your
pty. The data is only valid during the call, so copy it if you can't write it
right away.

@snippet c-vt-effects/src/main.c effects-write-pty

Full program: @ref c-vt-effects/src/main.c

### The bell

The bell has no data at all. This one counts bells in the userdata pointer.

@snippet c-vt-effects/src/main.c effects-bell

Full program: @ref c-vt-effects/src/main.c

### Title changes

The callback tells you the title changed, but not what it changed to. You can
read the new title with `GHOSTTY_TERMINAL_DATA_TITLE` inside the callback.

@snippet c-vt-effects/src/main.c effects-title-changed

Full program: @ref c-vt-effects/src/main.c

### Clipboard writes

A write is a request, not a command. You should ask the person using your
terminal unless `granted` is set, and the stream waits until your callback
returns while you do.

Answer by calling `reply` before you return. If you return without replying, the
write is denied. Every entry in `contents` is a representation of the same
value, so commit them together. A `contents_len` of zero asks you to clear the
clipboard.

@snippet c-vt-effects/src/main.c effects-clipboard-write

Full program: @ref c-vt-effects/src/main.c

### Clipboard reads

Reads work the same way. Ask first unless `granted` is set, and call `reply`
before you return. If you return without replying, the program gets an empty
clipboard for OSC 52 and `EPERM` for OSC 5522.

@snippet c-vt-effects/src/main.c effects-clipboard-read

Full program: @ref c-vt-effects/src/main.c

### Sequences we don't implement

The callback for unknown sequences has more to it than the others, so it has its
own guide. See @ref guide-custom-sequences.

## Putting it together {#guide-effects-register}

This registers the callbacks above, along with the unknown sequence callback,
and then writes a sequence for each one so you can watch them fire.

@snippet c-vt-effects/src/main.c effects-register

Full program: @ref c-vt-effects/src/main.c
