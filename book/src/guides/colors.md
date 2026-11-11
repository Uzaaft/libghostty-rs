# Colors {#guide-colors}

A terminal has a foreground color, a background color, a cursor color and a
256-color palette. You set them to your theme, and the programs running in the
terminal can change them.

That's the problem this page is about. A program can change the background with
an OSC sequence, but that shouldn't erase the theme you configured. So each
color has two layers: a default that you set, and an override that programs set
with OSC sequences, such as OSC 10, 11 and 12 for the foreground, background and
cursor, and OSC 4 for a palette entry.

If you only need to set a theme, read the first two sections. The rest is for
reading colors out of config files, generating palettes, and telling programs
whether you're in light or dark mode.

## Setting your theme {#guide-colors-defaults}

Use the various `Terminal::set_default_*` methods to set the default colors.
Passing `None` clears a default and leaves that color unset.

| Option                                 | Input type              | Description               |
| -------------------------------------- | ----------------------- | ------------------------- |
| [`Terminal::set_default_fg_color`] | `Option<Rgb>`      | Default foreground color  |
| [`Terminal::set_default_bg_color`] | `Option<Rgb>`      | Default background color  |
| [`Terminal::set_default_cursor_color`]     | `Option<Rgb>`      | Default cursor color      |
| [`Terminal::set_default_color_palette`]    | `Option<Palette>` | Default 256-color palette |

The palette is a little different. Passing `None` resets it to the built-in
default palette, and setting it keeps any per-index overrides that programs have
applied. Only the unmodified indices are updated.

```rs
{{#include ../../../example/rust-vt-colors/src/main.rs:colors-set-defaults}}
```

Full program: [rust-vt-colors/src/main.rs](/example/rust-vt-colors/src/main.rs)

## Reading colors {#guide-colors-reading}

Use the various getters to read colors. Each color has two keys. The
effective value is what you should draw with: the override if a program set one,
and the default if not. The default value ignores overrides.

| Data                                            | Output type             | Description                                    |
| ----------------------------------------------- | ----------------------- | ---------------------------------------------- |
| [`Terminal::fg_color`]        | `Option<Rgb>`      | Effective foreground (override or default)     |
| [`Terminal::bg_color`]        | `Option<Rgb>`      | Effective background (override or default)     |
| [`Terminal::cursor_color`]            | `Option<Rgb>`      | Effective cursor (override or default)         |
| [`Terminal::color_palette`]           | `Palette` | Current palette (with any OSC overrides)       |
| [`Terminal::default_fg_color`]| `Option<Rgb>`      | Default foreground only (ignores OSC override) |
| [`Terminal::default_bg_color`]| `Option<Rgb>`      | Default background only (ignores OSC override) |
| [`Terminal::default_cursor_color`]    | `Option<Rgb>`      | Default cursor only (ignores OSC override)     |
| [`Terminal::default_color_palette`]   | `Palette` | Default palette only (ignores OSC overrides)   |

The foreground, background and cursor getters return `None` if
there's neither a default nor an override. So handle that case, because a new
terminal starts with these unset. The palette getters always succeed, since the
palette always has a value. If nothing else is set, it's the built-in default.

```rs
{{#include ../../../example/rust-vt-colors/src/main.rs:colors-read}}
```

Full program: [rust-vt-colors/src/main.rs](/example/rust-vt-colors/src/main.rs)

Here are the two layers working together. A program overrides the foreground
with OSC 10, and then clearing the default leaves the override in place.

```rs
{{#include ../../../example/rust-vt-colors/src/main.rs:colors-main}}
```

Full program: [rust-vt-colors/src/main.rs](/example/rust-vt-colors/src/main.rs)

## Parsing colors {#guide-colors-parsing}

Your theme probably starts out as text. These functions parse the same color
strings as Ghostty's config and theme files, and none of them need a terminal.

[`Rgb::parse`] accepts:

- X11 color names, matched ASCII case-insensitively.
- 3- or 6-digit hex colors, with or without a leading `#`.
- 9- or 12-digit hex colors, with a leading `#`.
- XParseColor-style `rgb:<red>/<green>/<blue>` values.
- XParseColor-style `rgbi:<red>/<green>/<blue>` values.

Leading and trailing spaces and tabs are ignored. Use [`Rgb::parse_x11_color`]
when you only want to accept X11 names.

The [`FromStr`](std::str::FromStr) implementation on `Rgb` is identical to
[`Rgb::parse`], so you may use it instead if you have a color string.

```rs
let color = match Rgb::parse("ForestGreen") {
    Ok(color) => color,
    Err(e) => { /* Handle invalid color input. */ },
};

// ASCII byte strings are also accepted
let color = Rgb::parse(b"#abc").unwrap();

// Using `str::parse` / `FromStr::from_str` with XParseColor-style colors
let color = "rgb:12/34/56".parse::<Rgb>().unwrap();
```

[`X11ColorNames::new()`] returns the table of X11 names, if you want to offer
them to someone. The table is static memory that lives as long as the program.
Entries are in rgb.txt order. [`X11ColorNames::len()`] returns the number of entries.

```rs
// X11ColorNames implements IntoIterator, so you can directly use a for-loop
for name in X11ColorNames::new() {
    println!("{:?} = {:?}", name.name, name.color);
}
```

## Palettes {#guide-colors-palettes}

[`Palette::default()`] gives you the same 256-color palette the
terminal uses by default. [`Palette::parse_palette_entry()`] parses one Ghostty
palette override in `INDEX=COLOR` form. The index can be decimal or use a `0x`,
`0o` or `0b` prefix, and the color side accepts anything [`Rgb::parse()`]
does.

```rs
let mut palette = Palette::default();
if let Some((index, rgb)) = Palette::parse_palette_entry("0x10=#282c34") {
    palette.set(index, rgb);
}
```

[`Palette::generate()`] derives the 216-color cube and the grayscale
ramp from a base palette, a background and a foreground, so they match the
theme. Set bits in a [`PaletteMask`] to keep specific indices from the
base palette.

```rs
let palette = Palette::default();
let mut skip = PaletteMask::default();
skip.set(PaletteIndex(16));

let background = Rgb { r: 40, g: 44, b: 52 };
let foreground = Rgb { r: 220, g: 223, b: 228 };

let new_palette = Palette::generate(
    Some(&palette),
    Some(&skip),
    background,
    foreground,
    true, // harmonious
);
```

If your UI needs to make decisions from a color, [`Rgb::luminance()`],
[`Rgb::perceived_luminance()`] and [`Rgb::contrast()`] give you the
luminance and contrast values.

## Light and dark reports {#guide-colors-scheme}

Programs want to know if your terminal is light or dark, so they can pick colors
that fit. There are two ways they find out.

A program can ask with `CSI ? 996 n`. Register a callback with
[`Terminal::on_color_scheme()`] and fill in the current scheme, or
return `false` to ignore the query. See [Effects](effects.md).

Or a program can set mode 2031 and expect you to tell it when the scheme changes.
For this one, encode the report yourself with [`ColorScheme::encode()`]
and write it to the pty. Only send it if [`Mode::COLOR_SCHEME_REPORT`] is set,
which you read with [`Terminal::mode()`]. Dark schemes encode as `ESC [ ? 997 ; 1 n`
and light schemes as `ESC [ ? 997 ; 2 n`, which are the same bytes as the terminal's
reply to the query.

If your buffer is too small, the function returns [`Error::OutOfSpace`] and
sets the output parameter to the size it needs.

```rs
{{#include ../../../example/rust-vt-color-scheme/src/main.rs:color-scheme-report-encode}}
```

Full program: [rust-vt-color-scheme/src/main.rs](/example/rust-vt-color-scheme/src/main.rs)
