# Effects {#guide-effects}

An effect is a callback the terminal calls when a program asks for something the
terminal can't do by itself. That means things like ringing the bell, changing
the window title, writing to the clipboard, or answering a query.

Why callbacks? libghostty-vt has no window, no clipboard and no pty. Those are
yours. When a program writes a BEL we know what it means, but we can't make a
sound. So we tell you, and you decide what a bell is.

By default, the VT write functions only process sequences that directly change
terminal state. Anything with a side effect or a reply is ignored until you
register a handler for it. That includes queries, so a program that asks a
question gets no answer.

If you're only feeding bytes into a terminal to read the result back, you can
skip this page. You need effects as soon as a real program runs in the terminal.

## Registering an effect {#guide-effects-registering}

Each effect can be registered by implementing the corresponding **handler trait**
(a trait under the [`effect`] module prefixed with `On`) on a handler type.
Then install a handler object with the terminal with [`Terminal::register_effects`],
and call the corresponding **register function**, prefixed with `with_on_`:

```rust,ignore
use libghostty::{effect, Terminal};

struct Handler;
impl effect::OnBell for Handler {
    fn on_bell(&mut self, _terminal: &Terminal<'_>) {
        println!("Ring-a-ding-a-ding!");
    }
}

terminal
    .register_effects(Handler)?
    .with_on_bell()?;
```

The handlers always receive a shared reference to the [`Terminal`] as their
first parameter, as well as other parameters depending on the exact callback.

## The rules for handlers {#guide-effects-rules}

Effect handlers run synchronously, in the middle of [`Terminal::vt_write()`] or
[`Terminal::vt_write_until()`]. All of these rules come from that.

You can't write to the same terminal from an effect handler. The terminal is
still working through the first write, and it isn't reentrant. This applies
to both [`vt_write()`] and [`vt_write_until()`]. For this reason, callbacks
only receive a _shared_ reference to the terminal, which cannot call
mutating functions like [`vt_write()`].

[`vt_write()`]: Terminal::vt_write
[`vt_write_until()`]: Terminal::vt_write_until

Don't block, and don't do anything expensive. Further IO processing waits until
the handler function returns, so a slow handler is a slow terminal.

Copy what you want to keep. Strings and buffers passed to a handler are
borrowed, and they're only valid until it returns.

## The effects {#guide-effects-table}

There are 19 effects:

| Handler trait             | Trigger                                                             |
| ------------------------- | ------------------------------------------------------------------- |
| [`OnPtyWrite`]            | VT query and mode reports written back to the pty                   |
| [`OnBell`]                | BEL character (0x07)                                                |
| [`OnTitleChanged`]        | Title change via OSC 0 / OSC 2                                      |
| [`OnPwdChanged`]          | Pwd change via OSC 7 / OSC 9 / OSC 1337                             |
| [`OnEnquiry`]             | ENQ character (0x05)                                                |
| [`OnXtversion`]           | XTVERSION query (`CSI > q`)                                         |
| [`OnSize`]                | XTWINOPS query (`CSI 14/16/18/19 t`) or mode 2048 enable            |
| [`OnColorScheme`]         | Color scheme query (`CSI ? 996 n`)                                  |
| [`OnDeviceAttributes`]    | Device attributes query (`CSI c`, `CSI > c`, `CSI = c`)             |
| [`OnClipboardWrite`]      | Clipboard write via OSC 52 / OSC 1337 / OSC 5522                    |
| [`OnClipboardRead`]       | Clipboard read via OSC 52 `?` / OSC 5522                            |
| [`OnDesktopNotification`] | Desktop notification via OSC 9 / OSC 777                            |
| [`OnProgressReport`]      | Progress report via OSC 9;4                                         |
| [`OnProgramStatus`]       | Program status report via OSC 7501                                  |
| [`OnUnknownSequence`]     | APC or OSC sequence we don't implement. See [Custom Sequences].     |
| [`OnRenderHold`]          | Synchronized output (mode 2026) begins or ends                      |
| [`OnSemanticPrompt`]      | Shell reports a prompt or command step via OSC 133                  |
| [`OnReset`]               | Full reset (RIS, `ESC c`)                                           |
| [`OnResizeWindow`]        | Window resize request (`CSI 8 t`)                                   |

[Custom Sequences]: custom-sequences.md#guide-custom-sequences

## Common handlers {#guide-effects-common}

These are the handlers most terminals start with. Each one is from the same
example program, which prints what it receives.

### Replying to the program

Start with this one. Every reply the terminal generates comes through it, such
as the answer to a device status report or a mode query. Write the bytes to your
pty. The data is only valid during the call, so copy it if you can't write it
right away.

```rust,ignore
{{#include ../../example/rust-vt-effects/src/main.rs:effects-write-pty}}
```

Full program: [rust-vt-effects/src/main.rs](/example/rust-vt-effects/src/main.rs)

### The bell

The bell has no data at all. This one counts bells with a state variable
placed within a [`Cell`](std::cell::Cell), which allows for
[_interior mutability_](https://doc.rust-lang.org/reference/interior-mutability.html).

```rust,ignore
{{#include ../../example/rust-vt-effects/src/main.rs:effects-state-lifetime}}

    // ...

{{#include ../../example/rust-vt-effects/src/main.rs:effects-bell}}
```

Full program: [rust-vt-effects/src/main.rs](/example/rust-vt-effects/src/main.rs)

### Title changes

The handler tells you the title changed, but not what it changed to. You can
read the new title with [`Terminal::title`] inside the handler.

```rust,ignore
{{#include ../../example/rust-vt-effects/src/main.rs:effects-title-changed}}
```

Full program: [rust-vt-effects/src/main.rs](/example/rust-vt-effects/src/main.rs)

### Clipboard writes

A write is a request, not a command. You should ask the person using your
terminal unless `granted` is set, and the stream waits until your handler
returns while you do.

Answer by calling `reply` before you return. If you return without replying, the
write is denied. Every entry in `contents` is a representation of the same
value, so commit them together. A `contents_len` of zero asks you to clear the
clipboard.

```rust,ignore
{{#include ../../example/rust-vt-effects/src/main.rs:effects-clipboard-write}}
```

Full program: [rust-vt-effects/src/main.rs](/example/rust-vt-effects/src/main.rs)

### Clipboard reads

Reads work the same way. Ask first unless `granted` is set, and call `reply`
before you return. If you return without replying, the program gets an empty
clipboard for OSC 52 and `EPERM` for OSC 5522.

```rust,ignore
{{#include ../../example/rust-vt-effects/src/main.rs:effects-clipboard-read}}
```

Full program: [rust-vt-effects/src/main.rs](/example/rust-vt-effects/src/main.rs)

### Sequences we don't implement

The handler for unknown sequences has more to it than the others, so it has its
own guide. See [Custom Sequences](custom-sequences.md#guide-custom-sequences).

## Putting it together {#guide-effects-register}

This registers the handlers above, along with the unknown sequence handler,
and then writes a sequence for each one so you can watch them fire.

```rust,ignore
{{#include ../../example/rust-vt-effects/src/main.rs:effects-register}}
```

Full program: [rust-vt-effects/src/main.rs](/example/rust-vt-effects/src/main.rs)
