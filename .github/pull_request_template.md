## What changed and why

## Issue

Closes #

## How it was tested

Commands from the Commands section of [CLAUDE.md](../CLAUDE.md), with what you ran (pnpm from `src/ui/`, cargo from `src/`):

- [ ] `pnpm --filter koppel.ui typecheck`, `lint`, `test` and `build`, and `pnpm --filter @koppel/react-ui test`, if the UI changed
- [ ] `cargo build --workspace --locked`, and `cargo test --workspace --locked` in a Linux container (there is no CI)
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo fmt --all --check`
- [ ] `cargo deny check` and `.github/scripts/check-pins.sh`, if dependencies, licences or toolchain versions changed
- [ ] Not run, and why:

## Docs and changelog

- [ ] `docs/` changed (which file)
- [ ] `CHANGELOG.md` changed
- [ ] Neither needed, because:
