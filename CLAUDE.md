# Koppel

USB devices shared over the network. Read `docs/architecture.md` first.

## Layout

The root stays clean: no code, no build files. Everything that builds lives under `src/`: one
folder per crate, the UI and the installers.

| Folder | What |
|---|---|
| `docs/` | Architecture and decisions |
| `resources/design-system/` | The design system: brand book, `tokens.json`, component specs and previews, logos. The source of every colour, size and component style |
| `src/` | Shared build config: `.editorconfig`, the pnpm workspace, and the Rust workspace (`Cargo.toml`, see "The Rust workspace" in `docs/architecture.md`) |
| `src/koppel/` | The program (package `koppel`) and its `Dockerfile`, which builds the Linux binaries, the `.deb` files and the image |
| `src/koppel-*/` | The library crates: `koppel-config`, `koppel-web`, `koppel-usb`, `koppel-windows`, and `koppel-contract`, the contract tests |
| `src/ui/` | The UI, one pnpm workspace with `src/`: the React app (`app`, package `koppel.ui`) and the UI library (`react-ui`, package `@koppel/react-ui`) |
| `src/installer/` | The installers: `windows/` (WiX, `Koppel-Setup.exe`, with usbip-win2 vendored), `linux/` (nfpm, the `.deb` files), `docker/` (the compose files, `prepare-host.sh` and `deploy.sh`) and `notices/` (the third-party notices script and cargo-about's licence policy, shared by the Windows and Linux builds) |
| `src/tools/` | The design system scripts behind `pnpm design:sync` and `pnpm design:icons` |

## Rules

- **The UI never assumes a change worked.** Send it over REST, show it as pending, and treat it as done only when the live push (Server-Sent Events at `/api/events`) brings the value back after the server wrote its config file. Pushes are applied in `HubPushes` and pending state lives in `PendingProvider`, above the pages. See "The confirm rule" in `docs/architecture.md`.
- **Design values come from `resources/design-system/`.** Do not hand-edit `ui/react-ui/src/styles/tokens.css` or `components.css`; change the design system and run `pnpm design:sync`.
- **No new UI primitives in the app.** Add them to `@koppel/react-ui` and to the design system.
- **Every mutating fetch goes through `uiFetch`** (`api/http.ts`), which adds the `X-Koppel-UI` header the server requires. A route under `/api` on the UI port needs a session by default (`requires_sign_in` in `koppel-web/src/surface.rs`); list it among the anonymous paths only when it must work signed out. A route for remote clients goes in the listed surfaces on the network port. See "Ports and what they serve" in `docs/architecture.md`.
- **Program wiring goes in `koppel`**, everything else in the library crates. No crate depends on `koppel`.
- **GPL-3.0-or-later.** Code from projects under GPL-3.0, GPL-3.0-or-later, GPL-2.0-or-later, LGPL or a permissive licence may come in: keep its copyright and licence notice in the file and list it in `THIRD-PARTY-NOTICES.txt`. Never copy code from GPL-2.0-only projects (the Linux kernel, for one) or from projects without a licence.

## Commands

Run from `src/`:

```sh
pnpm install                                         # once
pnpm design:sync                                     # after changing resources/design-system
pnpm design:icons                                    # after changing the app icon
pnpm --filter @koppel/react-ui test                  # component tests
pnpm --filter koppel.ui typecheck
pnpm --filter koppel.ui lint
pnpm --filter koppel.ui test                         # app tests
pnpm --filter koppel.ui build                        # the UI the program embeds, needed for a release build
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p koppel                                  # https://localhost:5240; set Paths__Config and Paths__Keys to a folder of your own
pnpm --filter koppel.ui dev                          # https://localhost:61040, /api proxied to KOPPEL_UI_URL (https://127.0.0.1:5240)
bash koppel-contract/run-against-rust.sh             # the contract tests against a fresh build (Git Bash on Windows)
./installer/windows/build.ps1                        # PowerShell: Koppel-Setup.exe
./installer/linux/build.ps1 -Image                   # PowerShell: the .deb files and the image, loaded here, never pushed
```

Cargo builds from the committed `Cargo.lock`, and the installers pass `--locked`. The WiX projects restore
from their committed `packages.lock.json`; after changing a WiX package version in
`installer/windows/Directory.Packages.props`, run `dotnet restore` on both `.wixproj` files and commit the
lock files.

The sysfs tests create folders with `:` in their names, which Windows cannot, so they only run on
Linux. Run the workspace's tests in a Linux container as well (see "The Rust workspace" in
`docs/architecture.md` for clippy and rustfmt there):

```sh
docker run --rm -v "$PWD:/repo:ro" rust:1.99.1-trixie bash -c   "mkdir /work && tar -C /repo --exclude=./target --exclude=node_modules -cf - . | tar -C /work -xf - && cd /work && cargo test --workspace"
```
