The two Heartwood 0.2.0 packages the UI installs, from the private release
https://github.com/matumi-software/heartwood/releases/tag/v0.2.0 (fetched with `gh release download v0.2.0 -R matumi-software/heartwood`).
SHA256SUMS is the release file; `sha256sum -c --ignore-missing SHA256SUMS` checks the two .tgz files here.
Replace both files and run `pnpm install` in src/ui to move to a newer release.
