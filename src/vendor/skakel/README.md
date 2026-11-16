The Skakel crates Koppel uses, from the GitHub release
https://github.com/matumi-software/skakel/releases/tag/v0.3.0 (fetched with
`gh release download v0.3.0 -R matumi-software/skakel`).

- `*.crate` are the release's files, as built by `cargo package --workspace` from the tagged commit.
- `SHA256SUMS` is the release's file. `sha256sum -c --ignore-missing SHA256SUMS` checks the `.crate` files
  here; the release also carries two crates Koppel does not use (`skakel-vcam`, `skakel-vcam-source`).
- The folders are what each `.crate` file unpacks to. Cargo cannot take a `.crate` file as it is, so
  `[patch.crates-io]` in `src/Cargo.toml` names each folder by path, and the crates that depend on one another
  find the others there.
- Nothing here is edited. `.github/scripts/check-vendored-skakel.sh` checks the checksums and that each folder
  is exactly what its `.crate` file unpacks to.

To take a new Skakel release: download the release, check `SHA256SUMS`, replace the five `.crate` files and
`SHA256SUMS`, unpack each `.crate` here (`tar -xzf`, after removing the old folder), change the version in
the folder names, `[patch.crates-io]` and `[workspace.dependencies]`, run `cargo update`, and run the check
script.
