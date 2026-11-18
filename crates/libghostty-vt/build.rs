fn main() {
    // Rust's own `env!` macro doesn't support fallbacks, so we need
    // to do this in the build script instead
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_BOOK_URL");
    let url = std::env::var("LIBGHOSTTY_BOOK_URL")
        .unwrap_or_else(|_| "https://rust.libghostty.com".into());
    println!("cargo:rustc-env=BOOK_URL={url}");
}
