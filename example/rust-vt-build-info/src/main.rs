use libghostty_vt::{
    build_info::{self, Feature},
    error::Error,
};

// ANCHOR: build-info-query
fn query_build_info() -> Result<(), Error> {
    println!("SIMD: {}", build_info::supports_simd()?);
    println!("Kitty graphics: {}", Feature::KittyGraphics.is_supported()?);
    println!(
        "Tmux control mode: {}",
        build_info::supports_tmux_control_mode()?
    );

    println!("Version: {:?}", build_info::version_string());
    println!("Version major: {}", build_info::major_version()?);
    println!("Version minor: {}", build_info::minor_version()?);
    println!("Version patch: {}", build_info::patch_version()?);

    let pre_version = build_info::pre_version()?;
    if !pre_version.is_empty() {
        println!("Version pre  : {}", pre_version.escape_ascii());
    } else {
        println!("Version pre  : (none)");
    }

    let build_version = build_info::build_version()?;
    if !build_version.is_empty() {
        println!("Version build: {}", build_version.escape_ascii());
    } else {
        println!("Version build: (none)");
    }
    Ok(())
}
// ANCHOR_END: build-info-query

// ANCHOR: version-library
fn print_library_version() -> Result<(), Error> {
    // The full version, which is the easiest form to log or display. A
    // development build has a suffix, such as "1.1.0-dev+abc1234".
    println!("Library version: {:?}", build_info::version_string());

    // The same version as numbers, which is the easiest form to compare.
    let major = build_info::major_version()?;
    let minor = build_info::minor_version()?;
    let patch = build_info::patch_version()?;
    println!("Library version numbers: {major}.{minor}.{patch}");

    Ok(())
}
// ANCHOR_END: version-library

fn main() -> Result<(), Error> {
    query_build_info()?;
    print_library_version()?;

    Ok(())
}
