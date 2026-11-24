{
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "mdbookkit";
  version = "3.0.2";

  src = fetchFromGitHub {
    owner = "tonywu6";
    repo = "mdbookkit";
    # This applies to mdbook-permalinks as well
    tag = "mdbook-rustdoc-links-v${finalAttrs.version}";
    hash = "sha256-WvdQNVQpaHmWuj7ihLtMrnak3+uwl0r3hfBz7SG2Hfg=";
  };

  cargoHash = "sha256-d7IZQFCTX4nSy9nKArpkF2gYbLxtKr/XqUhFuDfIZ7o=";

  # Tests either require git or internet access
  doCheck = false;
})
