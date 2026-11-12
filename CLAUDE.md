# Koppel

USB devices shared over the network. Read `docs/architecture.md` first.

## Layout

The root stays clean: no code, no build files. Everything that builds lives under `src/`: one
folder per crate, the UI and the installers.

| Folder | What |
|---|---|
| `docs/` | Architecture and decisions |
| `resources/design-system/` | The design system: brand book, `tokens.json`, component specs and previews, logos. The source of every colour, size and component style |
| `src/` | Shared build config: `.editorconfig`, and the Rust workspace (`Cargo.toml`, see "The Rust workspace" in `docs/building.md`) |
| `src/koppel/` | The program (package `koppel`), and `tests/contract/`, the black-box tests that start it and drive it over HTTPS |
| `src/koppel-*/` | The library crates: `koppel-config`, `koppel-upgrade`, `koppel-web`, `koppel-usb`, and `koppel-windows` |
| `src/ui/` | The UI, one pnpm workspace (`package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml`): the React app (`app`, package `koppel.ui`) and the UI library (`react-ui`, package `@koppel/react-ui`) |
| `src/installer/` | The installers: `windows/` (WiX, `Koppel-Setup.exe`, with usbip-win2 vendored), `linux/` (nfpm, the `.deb` files), `docker/` (the `Dockerfile`, which builds the Linux binaries, the `.deb` files and the image, the compose files, `prepare-host.sh` and `deploy.sh`) and `notices/` (the third-party notices script and cargo-about's licence policy, shared by the Windows and Linux builds) |
| `src/ui/tools/` | The design system scripts behind `pnpm design:sync` and `pnpm design:icons` |

## Rules

- **The UI never assumes a change worked.** Send it over REST, show it as pending, and treat it as done only when the live push (Server-Sent Events at `/api/events`) brings the value back after the server wrote its config file. Pushes are applied in `LiveProvider` (`live/push-table.ts`) and pending state lives in `PendingProvider`, above the pages. See "The confirm rule" in `docs/ui.md`.
- **Design values come from `resources/design-system/`.** Do not hand-edit `ui/react-ui/src/styles/tokens.css` or `components.css`; change the design system and run `pnpm design:sync`.
- **No new UI primitives in the app.** Add them to `@koppel/react-ui` and to the design system.
- **Every mutating fetch goes through `uiFetch`** (`api/http.ts`), which adds the `X-Koppel-UI` header the server requires. A route under `/api` on the UI port needs a session by default (the protected group of `koppel-web/src/api/mod.rs`); put it in the anonymous group only when it must work signed out. A route for remote clients belongs to the network router (`koppel-web/src/routes.rs`). See "Ports and what they serve" in `docs/ports.md`.
- **Program wiring goes in `koppel`**, everything else in the library crates. No crate depends on `koppel`.
- **GPL-3.0-or-later.** Code from projects under GPL-3.0, GPL-3.0-or-later, GPL-2.0-or-later, LGPL or a permissive licence may come in: keep its copyright and licence notice in the file and list it in `THIRD-PARTY-NOTICES.txt`. Never copy code from GPL-2.0-only projects (the Linux kernel, for one) or from projects without a licence.

## Commands

Run the pnpm commands from `src/ui/` and the rest from `src/`:

```sh
# from src/ui/
pnpm install                                         # once
pnpm design:sync                                     # after changing resources/design-system
pnpm design:icons                                    # after changing the app icon
pnpm --filter @koppel/react-ui test                  # component tests
pnpm --filter koppel.ui typecheck
pnpm --filter koppel.ui lint
pnpm --filter koppel.ui test                         # app tests
pnpm --filter koppel.ui build                        # the UI the program embeds, needed for a release build
pnpm --filter koppel.ui dev                          # https://localhost:61040, /api proxied to KOPPEL_UI_URL (https://127.0.0.1:5240)

# from src/
cargo build --workspace
cargo test --workspace                              # runs the contract tests too: each starts koppel on a free port
cargo test -p koppel --test contract                # just the contract tests
KOPPEL_WRITE_WIRE=1 cargo test -p koppel-web wire    # after changing a type the API sends or reads: rewrites ui/app/src/api/generated/wire.ts
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check                                    # licences, advisories and sources, before a pull request (cargo install cargo-deny)
cargo run -p koppel                                  # https://localhost:5240; set KOPPEL_CONFIG_DIR and KOPPEL_KEYS_DIR to a folder of your own
./installer/windows/build.ps1                        # PowerShell: Koppel-Setup.exe
./installer/linux/build.ps1 -Image                   # PowerShell: the .deb files and the image, loaded here, never pushed
```

Cargo builds from the committed `Cargo.lock`, and the installers pass `--locked`. The WiX projects restore
from their committed `packages.lock.json`; after changing a WiX package version in
`installer/windows/Directory.Packages.props`, run `dotnet restore` on both `.wixproj` files and commit the
lock files.

The sysfs tests create folders with `:` in their names, which Windows cannot, so they only run on
Linux. Run the workspace's tests in a Linux container as well (see "The Rust workspace" in
`docs/building.md` for clippy and rustfmt there):

```sh
docker run --rm -v "$PWD:/repo:ro" rust:1.99.1-trixie bash -c   "mkdir /work && tar -C /repo --exclude=./target --exclude=./src/target --exclude=node_modules -cf - . | tar -C /work -xf - && cd /work/src && cargo test --workspace --locked"
```

## Pull requests

Open them from a short-lived branch into `main`. `.github/pull_request_template.md` asks what changed and why, the issue it closes, how it was tested (the commands above) and whether `docs/` or `CHANGELOG.md` changed. `.github/CODEOWNERS` names the maintainers who review every pull request; add a collaborator there when one joins.

There is no CI: no workflow runs on a hosted runner, so a pull request is checked by hand before it is opened, with the commands above: `cargo fmt --all --check`, `cargo build`, `cargo test` (in the Linux container) and `cargo clippy -- -D warnings`, all with `--locked`, the UI typecheck, lint and both test suites, and `cargo deny check` with `src/deny.toml` (licences, advisories, sources). `.github/scripts/check-pins.sh` checks that the toolchain versions and the two licence lists agree. A new licence goes into `src/deny.toml` and `src/installer/notices/about.toml` together. Dependencies are updated by hand: no bot proposes updates, so check `cargo update` and `pnpm outdated` from time to time.
