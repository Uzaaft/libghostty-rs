use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use bindgen::EnumVariation;
use bindgen::callbacks::{EnumVariantValue, IntKind, ItemInfo, ItemKind, ParseCallbacks};

use heck::ToShoutySnakeCase;
use quote::ToTokens;

// Match the vendored targets in build.rs, including the iOS xcframeworks.
// Clang spells the simulator and Windows GNU LLVM targets differently.
const TARGETS: &[(&str, &str)] = &[
    ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"),
    ("x86_64-unknown-linux-musl", "x86_64-unknown-linux-musl"),
    ("aarch64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"),
    ("aarch64-unknown-linux-musl", "aarch64-unknown-linux-musl"),
    ("aarch64-apple-darwin", "arm64-apple-darwin"),
    ("x86_64-apple-darwin", "x86_64-apple-darwin"),
    ("aarch64-apple-ios", "arm64-apple-ios"),
    ("aarch64-apple-ios-sim", "arm64-apple-ios-simulator"),
    ("x86_64-pc-windows-gnu", "x86_64-pc-windows-gnu"),
    ("aarch64-pc-windows-gnullvm", "aarch64-pc-windows-gnu"),
    ("x86_64-pc-windows-msvc", "x86_64-pc-windows-msvc"),
    ("aarch64-pc-windows-msvc", "aarch64-pc-windows-msvc"),
    ("aarch64-linux-android", "aarch64-linux-android"),
    ("x86_64-linux-android", "x86_64-linux-android"),
    ("wasm32-unknown-unknown", "wasm32-unknown-unknown"),
    ("wasm64-unknown-unknown", "wasm64-unknown-unknown"),
];

fn main() {
    // The headers this crate's build script installed for this very build,
    // so they always match the pin and the enabled features. GHOSTTY_SOURCE_DIR
    // or an explicit GHOSTTY_INCLUDE_DIR override them.
    let include_dir = if let Ok(dir) = env::var("GHOSTTY_INCLUDE_DIR") {
        PathBuf::from(dir)
    } else if let Ok(src) = env::var("GHOSTTY_SOURCE_DIR") {
        PathBuf::from(src).join("include")
    } else {
        let dir = PathBuf::from(option_env!("LIBGHOSTTY_VT_SYS_INCLUDE_DIR").unwrap_or_else(
            || {
                panic!(
                    "the build script installed no ghostty headers (pkg-config build?); \
                 set GHOSTTY_INCLUDE_DIR or GHOSTTY_SOURCE_DIR"
                )
            },
        ));
        assert!(
            dir.join("ghostty").join("vt.h").exists(),
            "no ghostty headers in {}; set GHOSTTY_INCLUDE_DIR or GHOSTTY_SOURCE_DIR",
            dir.display()
        );
        dir
    };

    let header = include_dir.join("ghostty").join("vt.h");
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));

    let builder = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.to_string_lossy()))
        // These headers only need freestanding C headers. Use Clang's builtins
        // instead of the host libc headers when parsing a different ABI.
        .clang_arg("-ffreestanding")
        // Wasm only adds utility functions; the C types are shared.
        .clang_arg("-D__wasm__")
        .allowlist_function("[Gg]hostty.*")
        .allowlist_type("[Gg]hostty.*")
        .allowlist_var("GHOSTTY_.*")
        // Only used to force enums to `int` size. It is defined as `INT_MAX`,
        // and whether bindgen can evaluate that depends on which `limits.h`
        // clang resolves, which differs between environments. Exclude it so
        // the output is the same everywhere.
        .blocklist_item("GHOSTTY_ENUM_MAX_VALUE")
        .generate_cstr(true)
        .derive_default(true)
        .size_t_is_usize(true)
        .layout_tests(true)
        .default_enum_style(EnumVariation::ModuleConsts)
        .parse_callbacks(Box::new(Callbacks));

    let mut declarations = None;
    let mut layouts: Vec<(String, Vec<syn::Meta>)> = Vec::new();
    for &(rust_target, clang_target) in TARGETS {
        let bindings = builder
            .clone()
            .clang_arg(format!("--target={clang_target}"))
            .generate()
            .unwrap_or_else(|error| panic!("failed to generate {rust_target} bindings: {error}"));
        let mut bindings =
            syn::parse_file(&bindings.to_string()).expect("bindgen must generate valid Rust");
        let mut checks = Vec::new();
        // Bindgen emits layout checks as anonymous consts. Keep them separate
        // so shared declarations never inherit the generator host's ABI.
        bindings.items.retain(|item| {
            if matches!(item, syn::Item::Const(item) if item.ident == "_") {
                checks.push(item.clone());
                false
            } else {
                true
            }
        });
        assert!(!checks.is_empty(), "no layout checks for {rust_target}");
        let checks = quote::quote!(#(#checks)*).to_string();
        let cfg = target_cfg(rust_target);
        // Share checks only when Clang actually produced identical layouts,
        // not just because two targets happen to have the same pointer width.
        if let Some((_, targets)) = layouts.iter_mut().find(|(value, _)| *value == checks) {
            targets.push(cfg);
        } else {
            layouts.push((checks, vec![cfg]));
        }

        let bindings = bindings.into_token_stream().to_string();
        if let Some(shared) = &declarations {
            assert_eq!(shared, &bindings, "declarations differ for {rust_target}");
        } else {
            declarations = Some(bindings);
        }
    }

    // Defining __wasm__ exposes these declarations to bindgen, but upstream
    // only exports the functions on Wasm. Bindgen's attribute callback only
    // supports types, so attach the target gates to the generated functions
    // here. Parse Rust rather than depending on bindgen's whitespace or docs.
    let mut bindings = syn::parse_file(&declarations.unwrap()).unwrap();
    for item in &mut bindings.items {
        if let syn::Item::ForeignMod(block) = item {
            for item in &mut block.items {
                if let syn::ForeignItem::Fn(function) = item
                    && function.sig.ident.to_string().starts_with("ghostty_wasm_")
                {
                    function
                        .attrs
                        .push(syn::parse_quote!(#[cfg(target_family = "wasm")]));
                }
            }
        }
    }
    write_generated(&manifest_dir.join("src/bindings.rs"), bindings);
    let mut checks = syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: Vec::new(),
    };
    for (index, (layout, targets)) in layouts.into_iter().enumerate() {
        let name = quote::format_ident!("layout_{index}");
        let items = syn::parse_file(&layout).unwrap().items;
        let cfg = if targets.len() == 1 {
            targets.into_iter().next().unwrap()
        } else {
            syn::parse_quote!(any(#(#targets),*))
        };
        checks.items.push(syn::parse_quote! {
            #[cfg(#cfg)]
            mod #name {
                use crate::bindings::*;
                #(#items)*
            }
        });
    }
    write_generated(&manifest_dir.join("src/layouts.rs"), checks);
}

fn target_cfg(target: &str) -> syn::Meta {
    // Ask rustc for the ABI identity; this needs no installed target std or SDK.
    // Do not include CPU features or atomics: these do not select C layouts.
    let output = Command::new("rustc")
        .args(["--print", "cfg", "--target", target])
        .output()
        .expect("failed to run rustc");
    assert!(output.status.success(), "failed to query cfg for {target}");
    let cfg = String::from_utf8(output.stdout).unwrap();
    let fields: Vec<syn::Meta> = cfg
        .lines()
        .filter(|line| {
            matches!(
                line.split_once('=').map(|(name, _)| name),
                Some(
                    "target_arch"
                        | "target_os"
                        | "target_env"
                        | "target_vendor"
                        | "target_abi"
                        | "target_pointer_width"
                        | "target_endian"
                )
            )
        })
        .map(|line| syn::parse_str(line).unwrap())
        .collect();
    syn::parse_quote!(all(#(#fields),*))
}

fn write_generated(out: &Path, file: syn::File) {
    std::fs::write(
        out,
        format!(
            "/* automatically generated by gen-bindings */\n\n{}",
            file.into_token_stream()
        ),
    )
    .unwrap_or_else(|error| panic!("failed to write bindings to {}: {error}", out.display()));
    // Keep doc attributes and the existing Rust formatting, rather than
    // rewriting all upstream documentation into block comments.
    let status = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(out)
        .status()
        .expect("failed to run rustfmt");
    assert!(status.success(), "rustfmt failed: {status}");
}

const PREFIXES: &[(&str, &str)] = &[
    ("GhosttySearchOption", "GHOSTTY_SEARCH_OPT"),
    ("GhosttySysOption", "GHOSTTY_SYS_OPT"),
    (
        "GhosttyTerminalUnknownSequenceTag",
        "GHOSTTY_TERMINAL_UNKNOWN_SEQUENCE",
    ),
    (
        "GhosttyTerminalResizeWindowTag",
        "GHOSTTY_TERMINAL_RESIZE_WINDOW",
    ),
    ("GhosttyOptimizeMode", "GHOSTTY_OPTIMIZE"),
    ("GhosttyKeyEncoderOption", "GHOSTTY_KEY_ENCODER_OPT"),
    ("GhosttyMouseTrackingMode", "GHOSTTY_MOUSE_TRACKING"),
    ("GhosttyMouseEncoderOption", "GHOSTTY_MOUSE_ENCODER_OPT"),
    ("GhosttySgrAttributeTag", "GHOSTTY_SGR_ATTR"),
    ("GhosttyOscCommandData", "GHOSTTY_OSC_DATA"),
    ("GhosttyOscCommandType", "GHOSTTY_OSC_COMMAND"),
    ("GhosttyOscOption", "GHOSTTY_OSC_OPT"),
    ("GhosttyTerminalOption", "GHOSTTY_TERMINAL_OPT"),
    (
        "GhosttyTerminalScrollViewportTag",
        "GHOSTTY_SCROLL_VIEWPORT",
    ),
    ("GhosttyStyleColorTag", "GHOSTTY_STYLE_COLOR"),
    ("GhosttyRowSemanticPrompt", "GHOSTTY_ROW_SEMANTIC"),
    ("GhosttyCellSemanticContent", "GHOSTTY_CELL_SEMANTIC"),
    ("GhosttyCellContentTag", "GHOSTTY_CELL_CONTENT"),
    ("GhosttySizeReportStyle", "GHOSTTY_SIZE_REPORT"),
    ("GhosttyModeReportState", "GHOSTTY_MODE_REPORT"),
    ("GhosttyFocusEvent", "GHOSTTY_FOCUS"),
    ("GhosttyResult", "GHOSTTY_"),
    ("GhosttyKittyGraphicsImageData", "GHOSTTY_KITTY_IMAGE_DATA"),
    (
        "GhosttySelectionGestureEventOption",
        "GHOSTTY_SELECTION_GESTURE_EVENT_OPT",
    ),
    (
        "GhosttySnapshotDecoderOption",
        "GHOSTTY_SNAPSHOT_DECODER_OPT",
    ),
    ("GhosttySemanticPromptKind", "GHOSTTY_SEMANTIC_PROMPT"),
    (
        "GhosttySemanticPromptPromptKind",
        "GHOSTTY_SEMANTIC_PROMPT_PROMPT",
    ),
];

#[derive(Debug)]
struct Callbacks;

impl ParseCallbacks for Callbacks {
    fn item_name(&self, item_info: ItemInfo) -> Option<String> {
        let prefix = match item_info.kind {
            // Do not rename functions since bindgen unconditionally prefixes
            // the `link_name` with `\u{1}`, which was supposed to stop LLVM
            // from mangling the name again but apparently this is necessary
            // on macOS and other Apple platforms?
            //
            // Honestly, what the hell. See:
            // https://github.com/rust-lang/rust-bindgen/issues/1221
            ItemKind::Function => return None,
            ItemKind::Var => "GHOSTTY_",
            _ => "Ghostty",
        };
        Some(item_info.name.trim_start_matches(prefix).to_string())
    }

    fn enum_variant_name(
        &self,
        enum_name: Option<&str>,
        original_variant_name: &str,
        _variant_value: EnumVariantValue,
    ) -> Option<String> {
        let enum_name = enum_name?;

        // Remove redundant C prefixes
        let prefix = PREFIXES
            .iter()
            .find(|(v, _)| *v == enum_name)
            .map(|(_, n)| n.to_string())
            .unwrap_or(enum_name.to_shouty_snake_case());

        let transformed = original_variant_name
            .trim_start_matches(&prefix)
            .trim_start_matches('_');

        Some(transformed.to_string())
    }

    fn process_comment(&self, comment: &str) -> Option<String> {
        Some(
            comment
                .lines()
                // Ignore doxygen directives.
                .filter(|s| !s.trim().starts_with("@"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    fn int_macro(&self, name: &str, _value: i64) -> Option<IntKind> {
        // Fixup some int macro types to reduce manual casting
        if name.starts_with("GHOSTTY_DA_") || name.starts_with("GHOSTTY_MODS_") {
            Some(IntKind::U16)
        } else if name.starts_with("GHOSTTY_KITTY_KEY_") || name.starts_with("GHOSTTY_COLOR_NAMED_")
        {
            Some(IntKind::U8)
        } else {
            None
        }
    }
}
