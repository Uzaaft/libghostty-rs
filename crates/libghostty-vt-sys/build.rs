use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(feature = "pkg-config")]
use std::process::Output;

/// Pinned ghostty commit. Update this to pull a newer version.
const GHOSTTY_REPO: &str = "https://github.com/ghostty-org/ghostty.git";
const GHOSTTY_COMMIT: &str = "01825411ab2720e47e6902e9464e805bc6a062a1";
#[cfg(feature = "pkg-config")]
const LIB_VERSION_PREFIX: &str = "0.1.0-dev";
#[cfg(feature = "pkg-config")]
const TYPE_LAYOUT_JSON: &str = include_str!("src/type_layout.json");
#[cfg(feature = "pkg-config")]
const REQUIRED_PKG_CONFIG_SYMBOLS: &[&str] = &[
    "ghostty_build_info",
    "ghostty_cell_get_multi",
    "ghostty_kitty_graphics_image_get_multi",
    "ghostty_kitty_graphics_placement_get_multi",
    "ghostty_kitty_graphics_placement_render_info",
    "ghostty_paste_encode",
    "ghostty_render_state_get_multi",
    "ghostty_render_state_row_cells_get_multi",
    "ghostty_render_state_row_get_multi",
    "ghostty_row_get_multi",
    "ghostty_sys_log_stderr",
    "ghostty_sys_set",
    "ghostty_terminal_get_multi",
    "ghostty_terminal_point_from_grid_ref",
    "ghostty_terminal_set",
    "ghostty_type_json",
];

fn main() {
    // docs.rs has no Zig toolchain. The checked-in bindings in src/bindings.rs
    // are enough for generating documentation, so skip the entire native
    // build when running under docs.rs.
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    println!("cargo:rerun-if-env-changed=GHOSTTY_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-env-changed=HOST");
    println!("cargo:rerun-if-changed=crates/libghostty-vt-sys/build.rs");
    #[cfg(feature = "pkg-config")]
    {
        println!("cargo:rerun-if-changed=crates/libghostty-vt-sys/src/bindings.rs");
        println!("cargo:rerun-if-changed=crates/libghostty-vt-sys/src/type_layout.json");
    }

    // An explicit source override should stay authoritative even when the
    // pkg-config feature is enabled, so local Ghostty checkouts remain easy to
    // test against.
    if env::var_os("GHOSTTY_SOURCE_DIR").is_some() {
        build_vendored();
        return;
    }

    // When the pkg-config feature is enabled, try pkg-config first. If the
    // library is already installed, validate that it still matches the checked-
    // in bindings before trusting it. If validation fails, fall back to the
    // vendored build so the crate keeps a single ABI source of truth.
    #[cfg(feature = "pkg-config")]
    if try_pkg_config() {
        return;
    }

    build_vendored();
}

/// Build libghostty-vt from source via zig. The zig build itself
/// generates a `libghostty-vt.pc` pkg-config file in `share/pkgconfig/`.
fn build_vendored() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let target = env::var("TARGET").expect("TARGET must be set");
    let host = env::var("HOST").expect("HOST must be set");

    // Locate ghostty source: env override > fetch into OUT_DIR.
    let ghostty_dir = match env::var("GHOSTTY_SOURCE_DIR") {
        Ok(dir) => {
            let p = PathBuf::from(dir);
            assert!(
                p.join("build.zig").exists(),
                "GHOSTTY_SOURCE_DIR does not contain build.zig: {}",
                p.display()
            );
            p
        }
        Err(_) => fetch_ghostty(&out_dir),
    };

    // Build libghostty-vt via zig.
    let install_prefix = out_dir.join("ghostty-install");

    let mut build = Command::new("zig");
    build
        .arg("build")
        .arg("-Demit-lib-vt")
        .arg("--prefix")
        .arg(&install_prefix)
        .current_dir(&ghostty_dir);

    // Only pass -Dtarget when cross-compiling. For native builds, let zig
    // auto-detect the host (matches how ghostty's own CMakeLists.txt works).
    if target != host {
        let zig_target = zig_target(&target);
        build.arg(format!("-Dtarget={zig_target}"));
    }

    run(build, "zig build");

    let lib_dir = install_prefix.join("lib");
    let include_dir = install_prefix.join("include");

    let lib_name = if target.contains("darwin") {
        "libghostty-vt.0.1.0.dylib"
    } else {
        "libghostty-vt.so.0.1.0"
    };

    assert!(
        lib_dir.join(lib_name).exists(),
        "expected shared library at {}",
        lib_dir.join(lib_name).display()
    );
    assert!(
        include_dir.join("ghostty").join("vt.h").exists(),
        "expected header at {}",
        include_dir.join("ghostty").join("vt.h").display()
    );

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=ghostty-vt");
    emit_include_metadata(&[include_dir]);
}

#[cfg(feature = "pkg-config")]
fn try_pkg_config() -> bool {
    let lib = match pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("libghostty-vt")
    {
        Ok(lib) => lib,
        Err(_) => return false,
    };

    if !lib.version.starts_with(LIB_VERSION_PREFIX) {
        println!(
            "cargo:warning=Ignoring pkg-config libghostty-vt {} because this crate expects {}*; falling back to the vendored build.",
            lib.version, LIB_VERSION_PREFIX
        );
        return false;
    }

    if !validate_pkg_config_library(&lib) {
        return false;
    }

    let lib = pkg_config::Config::new()
        .probe("libghostty-vt")
        .expect("pkg-config probe should succeed after validation");
    emit_include_metadata(&lib.include_paths);
    true
}

#[cfg(feature = "pkg-config")]
fn validate_pkg_config_library(lib: &pkg_config::Library) -> bool {
    let host = env::var("HOST").expect("HOST must be set");
    let target = env::var("TARGET").expect("TARGET must be set");

    // Validating via ghostty_type_json() requires running the discovered
    // library. That is only reliable for native builds, so cross-compiles keep
    // using the vendored path.
    if host != target {
        println!(
            "cargo:warning=Skipping pkg-config libghostty-vt because ABI validation only runs for native builds; falling back to the vendored build."
        );
        return false;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let probe_src = out_dir.join("pkg_config_probe.rs");
    let probe_bin = out_dir.join(format!("pkg_config_probe{}", std::env::consts::EXE_SUFFIX));

    std::fs::write(
        &probe_src,
        pkg_config_probe_source(&manifest_dir.join("src").join("bindings.rs")),
    )
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", probe_src.display()));

    let rustc = env::var("RUSTC").expect("RUSTC must be set");
    let mut compile = Command::new(rustc);
    compile
        .arg("--edition=2024")
        .arg("-A")
        .arg("warnings")
        .arg(&probe_src)
        .arg("-o")
        .arg(&probe_bin);

    for path in &lib.link_paths {
        compile.arg("-L").arg(format!("native={}", path.display()));
    }
    for path in &lib.framework_paths {
        compile
            .arg("-L")
            .arg(format!("framework={}", path.display()));
    }
    for file in &lib.link_files {
        if let (Some(dir), Some(stem)) = (file.parent(), dylib_stem(file)) {
            compile.arg("-L").arg(format!("native={}", dir.display()));
            compile.arg("-l").arg(format!("dylib={stem}"));
        }
    }
    for library in &lib.libs {
        compile.arg("-l").arg(format!("dylib={library}"));
    }
    for framework in &lib.frameworks {
        compile.arg("-l").arg(format!("framework={framework}"));
    }
    for ld_arg in &lib.ld_args {
        compile
            .arg("-C")
            .arg(format!("link-arg=-Wl,{}", ld_arg.join(",")));
    }

    let compile_output = run_output(compile, "rustc pkg-config probe");
    if !compile_output.status.success() {
        emit_command_failure_warning(
            "Ignoring pkg-config libghostty-vt because the ABI probe failed to link against it; falling back to the vendored build.",
            &compile_output,
        );
        return false;
    }

    let mut run = Command::new(&probe_bin);
    append_dynamic_library_search_path(&mut run, &target, &lib.link_paths);
    let run_output = run_output(run, "run pkg-config probe");
    if !run_output.status.success() {
        emit_command_failure_warning(
            "Ignoring pkg-config libghostty-vt because the ABI probe did not match the checked-in bindings; falling back to the vendored build.",
            &run_output,
        );
        return false;
    }

    true
}

#[cfg(feature = "pkg-config")]
fn pkg_config_probe_source(bindings_path: &Path) -> String {
    let symbol_refs = REQUIRED_PKG_CONFIG_SYMBOLS
        .iter()
        .map(|symbol| format!("        {symbol} as *const (),"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(nonstandard_style)]

include!({bindings_path:?});

use std::ffi::CStr;
use std::os::raw::c_char;

const EXPECTED_LAYOUT_JSON: &str = {type_layout_json:?};

fn main() {{
    let _required_symbols = [
{symbol_refs}
    ];

    let actual_layout_json = unsafe {{
        let ptr = ghostty_type_json();
        assert!(!ptr.is_null(), "ghostty_type_json returned a null pointer");
        CStr::from_ptr(ptr.cast::<c_char>())
            .to_str()
            .expect("ghostty_type_json must return UTF-8")
    }};
    assert_eq!(
        actual_layout_json,
        EXPECTED_LAYOUT_JSON,
        "ghostty_type_json output does not match the checked-in bindings"
    );
}}
"#,
        bindings_path = bindings_path,
        type_layout_json = TYPE_LAYOUT_JSON,
        symbol_refs = symbol_refs,
    )
}

fn emit_include_metadata(include_paths: &[PathBuf]) {
    if include_paths.is_empty() {
        return;
    }

    let joined = env::join_paths(include_paths)
        .unwrap_or_else(|error| panic!("failed to join include paths for cargo metadata: {error}"));
    println!("cargo:include={}", joined.to_string_lossy());
}

#[cfg(feature = "pkg-config")]
fn append_dynamic_library_search_path(command: &mut Command, target: &str, link_paths: &[PathBuf]) {
    if link_paths.is_empty() {
        return;
    }

    let key = if target.contains("darwin") {
        "DYLD_LIBRARY_PATH"
    } else if target.contains("windows") {
        "PATH"
    } else {
        "LD_LIBRARY_PATH"
    };

    let mut paths = link_paths.to_vec();
    if let Some(existing) = env::var_os(key) {
        paths.extend(env::split_paths(&existing));
    }

    let joined = env::join_paths(paths).unwrap_or_else(|error| {
        panic!("failed to join {key} entries for pkg-config probe: {error}")
    });
    command.env(key, joined);
}

#[cfg(feature = "pkg-config")]
fn dylib_stem(path: &Path) -> Option<&str> {
    let file_name = path.file_name()?.to_str()?;
    if let Some(stem) = file_name
        .strip_prefix("lib")
        .and_then(|name| name.split('.').next())
    {
        return Some(stem);
    }

    path.file_stem()?.to_str()
}

#[cfg(feature = "pkg-config")]
fn emit_command_failure_warning(message: &str, output: &Output) {
    println!("cargo:warning={message}");

    for line in String::from_utf8_lossy(&output.stderr).lines() {
        if !line.trim().is_empty() {
            println!("cargo:warning={line}");
        }
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if !line.trim().is_empty() {
            println!("cargo:warning={line}");
        }
    }
}

/// Clone ghostty at the pinned commit into OUT_DIR/ghostty-src.
/// Reuses an existing clone if the commit matches.
fn fetch_ghostty(out_dir: &Path) -> PathBuf {
    let src_dir = out_dir.join("ghostty-src");
    let stamp = src_dir.join(".ghostty-commit");

    // Skip fetch if we already have the right commit.
    if stamp.exists()
        && let Ok(existing) = std::fs::read_to_string(&stamp)
        && existing.trim() == GHOSTTY_COMMIT
    {
        return src_dir;
    }

    // Clean and clone fresh.
    if src_dir.exists() {
        std::fs::remove_dir_all(&src_dir)
            .unwrap_or_else(|e| panic!("failed to remove {}: {e}", src_dir.display()));
    }

    eprintln!("Fetching ghostty {GHOSTTY_COMMIT} ...");

    let mut clone = Command::new("git");
    clone
        .arg("clone")
        .arg("--filter=blob:none")
        .arg("--no-checkout")
        .arg(GHOSTTY_REPO)
        .arg(&src_dir);
    run(clone, "git clone ghostty");

    let mut checkout = Command::new("git");
    checkout
        .arg("checkout")
        .arg(GHOSTTY_COMMIT)
        .current_dir(&src_dir);
    run(checkout, "git checkout ghostty commit");

    std::fs::write(&stamp, GHOSTTY_COMMIT).unwrap_or_else(|e| panic!("failed to write stamp: {e}"));

    src_dir
}

fn run(mut command: Command, context: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("failed to execute {context}: {error}"));
    assert!(status.success(), "{context} failed with status {status}");
}

#[cfg(feature = "pkg-config")]
fn run_output(mut command: Command, context: &str) -> Output {
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to execute {context}: {error}"))
}

fn zig_target(target: &str) -> String {
    let value = match target {
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        "x86_64-unknown-linux-musl" => "x86_64-linux-musl",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        "aarch64-unknown-linux-musl" => "aarch64-linux-musl",
        "aarch64-apple-darwin" => "aarch64-macos-none",
        "x86_64-apple-darwin" => "x86_64-macos-none",
        other => panic!("unsupported Rust target for vendored build: {other}"),
    };
    value.to_owned()
}
