# Building, releasing and testing

## Building and releasing

Everything builds from committed lock files: Rust from `src/Cargo.lock` (`--locked`), the UI from
`src/ui/pnpm-lock.yaml` (`--frozen-lockfile`), and the WiX projects from their `packages.lock.json`
(`-p:RestoreLockedMode=true`). WiX is an MSBuild SDK, so the Windows installer also needs the .NET SDK; its
package versions are in `src/installer/windows/Directory.Packages.props`, and after changing one, run
`dotnet restore` on both `.wixproj` files and commit the lock files. The Windows installer is described in
["Installer and Windows service"](installers.md#installer-and-windows-service), and the Linux binaries, packages and image in "Linux packages and
the image in Rust" below.

Every pull request runs `.github/workflows/ci.yml`. The four checks the ruleset requires are Rust (format, build, test and clippy with `--locked`) on Linux and Windows, the UI (typecheck, lint, tests) and `cargo deny check` against `src/deny.toml`, which allows the licences `src/installer/notices/about.toml` accepts and fails on an advisory or an unknown source. Three more jobs run beside them: Release build (`pnpm build` of the UI, then `cargo build --release --locked -p koppel`, so the size profile and the embedded UI are built as shipped), Linux packages and image (the Dockerfile's `linux-packages` target and the image for the three platforms, which also builds the three musl targets) and Pins (`.github/scripts/check-pins.sh`: the Rust, Node and pnpm versions agree across `src/rust-toolchain.toml`, `src/ui/.nvmrc`, `package.json`, the Dockerfile and the docs, and `deny.toml` and `about.toml` list the same licences). Every action is pinned by commit SHA with its version in a comment, and every job has a `timeout-minutes`.

The Rust version lives in `src/rust-toolchain.toml` and Node in `src/ui/.nvmrc`; change them there, and then in the places `check-pins.sh` names. `.github/workflows/advisories.yml` runs every Monday: `cargo deny check advisories`, `pnpm audit --prod`, and a check for a newer Skakel tag, which opens an issue. A failed run emails the repository's Actions watchers. Dependencies are updated by hand; no bot proposes updates.

To release:

1. Set the version under `[workspace.package]` in `src/Cargo.toml`. The Windows installer, the `.deb`
   files and the image all take it from there.
2. Set the image tag in the three composes in `src/installer/docker` (`docker-compose.yml`,
   `docker-compose.windows-client.yml`, `docker-compose.windows-server.yml`), in the build command in
   `docs/windows.md` and in the README. They name the released version.
3. For a new usbip-win2 version, run `src/installer/windows/export-usbip-win2.ps1` (it rewrites `SHA256SUMS`; `-SumsOnly` regenerates only that), then add the new version to `$usbipWin2Pins` in `src/installer/windows/build.ps1` (signer thumbprints and the hashes of the two unsigned DLLs, taken from a copy you checked by hand: `SHA256SUMS` alone could be regenerated to match a swapped file) and check which CLI11 tag its licence comes from.
4. Add the version to `CHANGELOG.md`.
5. Build and push the image for all three platforms, from `src/`, with the version and `latest`:

   ```sh
   docker buildx build -f installer/docker/Dockerfile --platform linux/amd64,linux/arm64,linux/arm/v6 \
     -t matumisoftware/koppel:<version> -t matumisoftware/koppel:latest --push .
   ```
6. Build the three `.deb` files with `src/installer/linux/build.ps1` and `Koppel-Setup.exe` with
   `src/installer/windows/build.ps1`, and publish them. Record the image size per platform in the release
   notes.

## The Rust workspace

Koppel 0.2.0 is a rewrite in Rust (milestone v0.2.0, #82 to #92) of the C# Koppel 0.1.x, whose source
stays at tag `v0.1.2`. `src/` is the Cargo workspace: `src/Cargo.toml`, the committed `src/Cargo.lock`,
and the build output in `src/target/`. It follows Skakel's: edition 2024, Rust 1.99, and every
third-party crate pinned in `[workspace.dependencies]` with a note on its licence. `[workspace.lints]` forbids `unsafe_code`; `koppel` and `koppel-contract` take it with `[lints] workspace = true`, and the crates that call the operating system (`koppel-config`, `koppel-usb`, `koppel-web`, `koppel-windows`) do not yet, because each has unsafe blocks.

| Crate | What |
|---|---|
| `koppel` | The program |
| `koppel-config` | The files on disk: the config files with 0.1.x's JSON and write rules, the secret files, the server certificate, the folder hardening, and the conversion of a 0.1.x install (see ["Config files in Rust"](config.md#config-files-in-rust)) |
| `koppel-web` | The web server: axum on tokio, TLS wired by hand through tokio-rustls, HTTP/1.1 and HTTP/2 by ALPN, the plain HTTP redirect, `/healthz`, the UI, and the UI's server side: sign-in, the REST API and the live push (see ["The UI's server side in Rust"](ui.md#the-uis-server-side-in-rust)) |
| `koppel-contract` | Contract tests that drive a running Koppel over HTTPS (see ["Contract tests"](#contract-tests) below). Not shipped |
| `koppel-usb` | The USB layer: USB/IP, the Linux kernel's drivers through sysfs, usbip-win2, usbipd-win, and the server and client roles over a stream to the other machine (see ["USB in Rust"](usb.md#usb-in-rust)) |
| `koppel-windows` | The Windows app: the service, the event log, the tray launcher, and `koppel-desktop` (installed as `KoppelDesktop.exe`), the tray icon and the Start menu entry (see ["Installer and Windows service"](installers.md#installer-and-windows-service)). Windows only; elsewhere it builds to an empty library and a program that does nothing. `koppel` calls it in three places: `--restart-service`, connecting to the service manager, and the event log layer |

- **Certificate:** each handshake asks `CertificateSlot` for the current certificate, and replacing it takes effect from the next connection. Session resumption is off (no tickets, no session cache), so every connection presents the current certificate, which Skakel's handover relies on. The program presents the one in `secrets.json`. On a first start it makes one as 0.1.x did (self-signed ECDSA P-256, ten years, the machine's name and `localhost`) and keeps it there as a PKCS#12 in 0.1.x's shape (`koppel_config::pkcs12::write`), with a token signing key, which is ECDSA P-256 as Skakel's Rust server signs. A 0.1.0 `secrets.json` is carried over, or copied aside when it cannot be.
- **UI:** `koppel-web/build.rs` reads Vite's output in `src/ui/app/dist`, gzips every file that shrinks by 5% or more, and rust-embed puts one copy of each file into the program. The gzip copy goes out as it is to a client that takes gzip (every browser) and is inflated for one that does not. Files under `assets/` have a hash in their names and are sent with `max-age=31536000, immutable`; index.html and the rest with `no-cache`. A path whose last part has no dot gets index.html, except under `/api`.
- **No UI built:** a debug build (`cargo build`, `test`, `clippy`) embeds a placeholder page and prints a warning, so the Rust side builds without Node. A release build stops with an error that says how to build the UI.

Run from `src/`:

```sh
pnpm --dir ui install && pnpm --dir ui --filter koppel.ui build   # the UI the program embeds (src/ui/app/dist), needed for a release build
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo test -p koppel-windows -- --ignored          # Windows: starts KoppelDesktop.exe's tray icon for a moment
cargo run -p koppel                             # https://localhost:5240, Ctrl+C stops it; set Paths__Config and Paths__Keys to a folder of your own
```

The same on Linux, in a container. The image has no clippy or rustfmt; add them with
`rustup component add clippy rustfmt` first to run those two there.

```sh
docker run --rm -v "$PWD:/repo:ro" rust:1.99.1-trixie bash -c \
  "mkdir /work && tar -C /repo --exclude=./target --exclude=node_modules -cf - . | tar -C /work -xf - && cd /work && cargo test --workspace"
```

### Release profile and sizes

`[profile.release]` in `src/Cargo.toml` builds for size (#92): `opt-level = "z"`, fat LTO, one codegen unit
and `strip = true`, with ring alone at opt-level 3. The same code runs on a Raspberry Pi Zero (ARMv6, one
core, 512 MB), so each candidate was timed as well as weighed:

| Profile | `Koppel.exe` | PBKDF2, x86-64 | TLS, x86-64 | HTTP/2, x86-64 | PBKDF2, ARM1176 | TLS, HTTP/2, ARM1176 |
|---|---|---|---|---|---|---|
| Cargo's defaults (3, no LTO, 16 units) | 17.4 MB | 0.068 s | 1995 MiB/s | 1455 MiB/s | | |
| 3, thin LTO | 15.4 MB | 0.064 s | 2053 MiB/s | 1466 MiB/s | | |
| 3, fat LTO, 16 units | 16.2 MB | 0.062 s | 2098 MiB/s | 1478 MiB/s | | |
| 3, fat LTO | 14.3 MB | 0.064 s | 2220 MiB/s | 1632 MiB/s | 0.955 s | 9, 9 MiB/s |
| s, thin LTO | 9.8 MB | 0.080 s | 2076 MiB/s | 1513 MiB/s | | |
| s, fat LTO | 9.2 MB | 0.081 s | 2384 MiB/s | 1551 MiB/s | 1.068 s | 8, 8 MiB/s |
| s, fat LTO, ring at 3 | 9.2 MB | 0.069 s | 1852 MiB/s | 1452 MiB/s | 0.965 s | 9, 9 MiB/s |
| z, fat LTO | 8.4 MB | 0.110 s | 1644 MiB/s | 1205 MiB/s | 1.800 s | 8, 8 MiB/s |
| **z, fat LTO, ring at 3** | **8.5 MB** | **0.069 s** | **2159 MiB/s** | **1517 MiB/s** | **1.000 s** | **10, 9 MiB/s** |

One codegen unit and `strip = true` in every row but the first. The speed check was a throwaway program on a
single-threaded tokio runtime, as on the Zero's one core: the 600,000-round PBKDF2 of a sign-in through ring
as `auth/password.rs` calls it, 256 MiB over TLS 1.3 (tokio-rustls on ring) on loopback, and 256 MiB as one
HTTP/2 response over TLS (hyper; Skakel's gRPC streams run on HTTP/2). Each figure is the median of five runs.
The ARM1176 columns are the same program for `arm-unknown-linux-musleabihf` under `qemu-arm -cpu arm1176`, the
Zero's CPU, with 8 MiB per stream: emulated, so only the ratios between rows mean anything. The x86-64 figures
move by about 10% from run to run.

- **Why `z`:** it is the smallest, and with ring at 3 it is as fast as opt-level 3 on everything measured.
  ring is where `z` hurt: its C code and the Rust around its assembly, so PBKDF2 took 1.8 times as long and
  TLS was slower. The per-package override costs 60 KB and wins all of that back.
- **Why fat LTO and one unit:** fat LTO is 0.6 to 1.1 MB smaller than thin, and 16 units undo most of it. The
  cost is build time: the Windows programs build from scratch in about 2 minutes instead of 45 seconds. Debug
  builds, tests and clippy are unchanged.
- **No `panic = "abort"`:** Koppel relies on unwinding. koppel-config runs each change handler as its own
  tokio task and logs one that panics while the rest still run (`file.rs`, `deliver`, tested by
  `a_handler_that_panics_does_not_stop_the_others`); koppel-usb's `BackgroundTasks` and listener lists do the
  same (`tasks.rs`); and the web server runs each connection as its own task (koppel-web's `server.rs`), so
  a panic while serving one ends that connection, not the process.
- **Dependencies:** unchanged. Koppel's own entries already leave out the default features it does not use.
  What is big (`cargo bloat --crates` on the old Windows build, 11.4 MiB of code in a 16.6 MiB file):
  koppel-web 2.9 MiB, std 1.5, reqwest 0.8, koppel-usb 0.8, skakel-client 0.8, rustls 0.7, tokio 0.6,
  skakel-server 0.6, koppel-config 0.4, h2 0.3, hyper 0.2. The only crates in the programs twice are base64
  (0.22 for tonic, 0.23 for the rest) and getrandom (0.2 for ring, 0.4 for Skakel), neither of which
  Koppel can drop; syn 2 and 3 run only at build time. reqwest comes in through skakel-client, and with it
  url and its IDNA tables, tower-http's redirect following and rustls-platform-verifier; whether Skakel
  needs those is Skakel's call.

Sizes, before and after this profile, in MB of a million bytes (Windows Explorer shows MiB, so 33.4 MB shows
as 31.9 MB there):

| | 0.1.2 | Before | Now |
|---|---|---|---|
| `Koppel.exe` | | 17.3 MB | 8.4 MB |
| `KoppelDesktop.exe` | | 0.76 MB | 0.34 MB |
| Installed (the programs, usbip-win2, the notices) | 117 MB | 21.0 MB | 11.7 MB |
| `Koppel-Setup.exe` (25.6 MB of it the Visual C++ runtime) | 69 MB | 33.4 MB | 31.0 MB |
| Linux binary, amd64 | | 14.7 MB | 6.7 MB |
| Linux binary, arm64 | | 13.8 MB | 5.9 MB |
| Linux binary, armv6 | | 13.1 MB | 5.7 MB |
| `.deb`, amd64 | 53.8 MB | 6.6 MB | 3.7 MB |
| `.deb`, arm64 | 50.9 MB | 6.5 MB | 3.6 MB |
| `.deb`, armhf | none | 6.1 MB | 3.3 MB |
| Image, compressed, amd64 | 102.1 MB | 7.5 MB | 4.5 MB |
| Image, compressed, arm64 | 98.0 MB | 7.4 MB | 4.5 MB |
| Image, compressed, arm/v6 | none | 7.1 MB | 4.2 MB |
| Image on disk, amd64 | | 28.4 MB | 17.4 MB |

"Before" is main at 66970af (Skakel v0.2.0, served on Koppel's own ports) with Cargo's default release
profile, and "Now" the same code with this one. The profile comparison above was measured a few commits
earlier (501c9f6), which is why its first row says 17.4 MB. The Windows figures come from
`src/installer/windows/build.ps1` (installed is the staging folder the MSI installs), the Linux ones from
`src/installer/linux/build.ps1 -Image` and the Dockerfile's `binaries` target; the image sizes are what
`docker image ls --tree` reports.

### Linux packages and the image in Rust

`src/installer/docker/Dockerfile` builds Koppel for Linux with Docker alone (#89, #90). It builds the UI
with pnpm in the node image, then cross-compiles static musl binaries with
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild), which uses Zig as the C compiler and
linker (ring has C and assembly), for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` and
`arm-unknown-linux-musleabihf`. That last one is ARMv6 with hardware floating point (VFPv2, Thumb-1), which
every 32-bit Raspberry Pi runs, the Zero and the Pi 1 included. Everything compiles on the build machine's
own architecture, so nothing runs under emulation, and one build serves all three platforms.

- **Pins:** every image by tag and digest (node, rust, nfpm, distroless), cargo-zigbuild, cargo-about and
  pnpm by version, Zig and PowerShell by version and checksum. The Dockerfile's header says how to bump them.
- **Why cargo-zigbuild:** `cross` runs a container per target, so inside a Docker build it needs Docker in
  Docker, and its last release is from 2023. Per-target gcc toolchains mean a musl cross compiler for each
  target to fetch and pin; Debian has none for ARMv6. One Zig covers every target.
- **Targets:** `--target binaries` writes `<arch>/koppel` (amd64, arm64, and arm for ARMv6),
  `--target linux-packages` the `.deb` files, and the last stage is the image. Cargo's registry, Zig's
  cache and `target/` are cache mounts, kept between builds.
- **Packages:** `src/installer/linux/build.ps1` builds `koppel_<version>_amd64.deb`, `_arm64.deb` and
  `_armhf.deb` (the ARMv6 binary, which is what 32-bit Raspberry Pi OS installs) into
  `src/installer/linux/bin`, with nfpm and the files beside it. The version is the workspace version in
  `src/Cargo.toml`. `-Image` also builds the image for `linux/amd64`, `linux/arm64` and `linux/arm/v6` and
  loads it into this machine's image store (Docker's containerd store, the default since Engine 29). It
  never pushes.
- **What a .deb holds:** the layout of 0.1.x, so an upgrade from it replaces 0.1.x's files and keeps
  `/var/lib/koppel` (the first start converts the 0.1.x secret files, see ["Config files in Rust"](config.md#config-files-in-rust)):
  `/usr/lib/koppel/koppel` with `/usr/bin/koppel` linked to it, `koppel.service`,
  `/usr/lib/modules-load.d/koppel.conf`, and the same install, remove and purge scripts as 0.1.x. It depends
  on nothing, since the binary is static. New in it: the applications menu entry (#68), the systemd
  watchdog and hardening (#71, #72) below, and the notices and copyright file (#100).
- **Notices and copyright (#100):** `/usr/share/doc/koppel/THIRD-PARTY-NOTICES.txt`, in the `.deb` and in
  the image, comes from the Windows installer's script and licence policy (`src/installer/notices`), so both
  list the same way: Koppel's licence, every crate in the binaries with its licence text, and the npm
  packages the UI bundles. The Dockerfile's `crate-licences` stage runs cargo-about with `about.toml` for the
  three Linux targets, where cargo and the crates' sources are, and the `notices` stage runs
  `new-notices.ps1 -AboutJson` under PowerShell for Linux on top of the UI stage, where pnpm and the UI's
  packages are. The build context is `src/`, so Koppel's licence text comes from Debian's
  `/usr/share/common-licenses/GPL-3`, which is the same text as the repo's `LICENSE`. One file covers all
  three architectures. `/usr/share/doc/koppel/copyright` (`src/installer/linux/copyright`) is in Debian's
  machine-readable format: Koppel under GPL-3.0-or-later, and a pointer to the notices for the third-party
  code inside the binary. `/usr/bin/koppel` is a relative link, and a lintian override says the binary is
  static on purpose. The maintainer is the GitHub noreply address. Debian 13's lintian still reports no
  `changelog.gz`, `systemctl` called straight from the scripts rather than through `deb-systemd-helper`, and
  no man page.
- **Applications menu (#68):** `/usr/share/applications/koppel.desktop` with Koppel's icon in
  `/usr/share/icons/hicolor` (16 to 256 px and the SVG, rendered by `pnpm design:icons` into
  `src/installer/linux/icons`). It runs `/usr/lib/koppel/koppel-open`, which opens
  `https://localhost:<port>/` through `xdg-open`, with the port from `Ports__Ui` in `/etc/default/koppel`, or
  5240. The package only suggests `xdg-utils`, so a server without a desktop gets nothing extra.
- **Watchdog (#71):** `WatchdogSec=30`. Koppel (`koppel/src/systemd.rs`, on the
  [sd-notify](https://crates.io/crates/sd-notify) crate) sends `READY=1` once its ports listen,
  `STOPPING=1` when it stops, and `WATCHDOG=1` every half interval, but only when its own `/healthz`
  answers within a quarter interval. The health check goes through the router and its guard in process, on
  the same runtime as every connection, so a runtime with every worker stuck, or a lock that never comes
  free, stops the heartbeat. Without `WATCHDOG_USEC` (Docker, Windows, a console) none of it runs.
- **Hardening (#72):** still root, which the files in `/sys` need, with what Koppel does not use taken
  away: `ProtectSystem=strict` with `/var/lib/koppel` writable, no capabilities but
  `CAP_NET_BIND_SERVICE`, `/dev` limited to the usbfs nodes the device reset opens, a system call filter,
  and the rest of the usual list. `koppel.service` says for each setting left out what it would break
  (`ProtectKernelTunables` makes `/sys` read-only, `PrivateDevices` hides `/dev/bus/usb`, `PrivateUsers`
  loses root's ownership of `/sys`). `systemd-analyze security koppel` scores 2.1 (OK), down from 9.4
  (UNSAFE) for 0.1.x's unit. A config folder moved out of `/var/lib/koppel` needs a `ReadWritePaths=` for it
  in a drop-in.
- **Image (#90):** `gcr.io/distroless/static-debian13:nonroot`: root certificates, time zone data, the
  nonroot user (65532), no shell and no libc, with Koppel at `/usr/local/bin/koppel` and the `/koppel`
  volume as in 0.1.x. distroless publishes arm only as `arm/v7`, but static holds no programs, so the
  `arm/v6` image takes those files as they are (its package list then says armhf).
- **Health check (#100):** the image has no shell or curl, so `HEALTHCHECK` runs `koppel --healthcheck`
  (`koppel/src/healthcheck.rs`), as 0.1.x's image did, with the same timings (every
  30 s, 10 s timeout, 40 s start period, 3 retries). It starts no server: it reads `Ports:Ui` and
  `Ports:Address` as the server does, sends `GET /healthz` over HTTPS to that address (loopback for an
  any-address), then to 127.0.0.1, and exits 0 on 200 and 1 otherwise. An address that cannot be reached
  passes to the next; one that takes the connection and does not answer within 5 s fails the check. The
  certificate is not checked, as in 0.1.x: it is Koppel's own self-signed one, the probe sends nothing
  secret, and it only asks whether Koppel is up. The client is hyper's HTTP/1.1 half on the rustls the
  server already carries, so no crate was added.
- **Composes:** the ones in `src/installer/docker` name the released image,
  and the image name and run options did not change from 0.1.x. `deploy.sh` builds this
  Dockerfile.

The binary, `.deb` and image sizes per architecture, against 0.1.2, are in ["Release profile and sizes"](#release-profile-and-sizes) above.

Tested in containers: each `.deb` installs in Debian 13 of its architecture (arm64 and armhf under QEMU, the
armhf binary run as an ARM1176 CPU, the Pi Zero's, on which Debian's own ARMv7 programs stop on an illegal
instruction) and Koppel answers `/healthz` and serves the UI; the amd64 package upgrades 0.1.2 with
`/var/lib/koppel` unchanged; under systemd in a container the hardened unit starts, passes
`systemd-analyze verify`, and a Koppel frozen with `kill -STOP` is restarted by the watchdog; the image runs
on all three platforms, as nonroot and as root on a 0.1.2 volume. Not tried yet: real hardware (a Pi Zero,
sharing and attaching through the hardened unit) and a desktop's applications menu.

### Contract tests

`koppel-contract` checks that the Rust build answers like 0.1.x (#91). Its tests drive a running Koppel
from the outside, over HTTPS, the way the browser and other Koppel machines do: every REST endpoint with
its errors, sign-in with its cookie, lockout and limits, the guards on each port and role, and pairing
between two Koppels. They check status codes, field names and shapes and behaviour, not times or ids. The
crate talks HTTP only and never links Koppel's own crates.

- **Targets:** environment variables name the Koppels to test, one for most tests and a few more for states only one test may touch (a fresh machine, a closed setup window, `Ui:AllowFrom=local`, the rate limit). The crate's docs list them. Without them every test says it was skipped and passes, so `cargo test --workspace` needs no Koppel running.
- **Never an install someone uses:** the tests change settings, the password, clients and the certificate.
- **Certificate:** the first certificate a target presents is pinned for the run, and every later connection must present it or one a test pinned before switching. The name in it is not checked, since the tests reach the target as `localhost`.
- **The live push:** `koppel_contract::push` lists the ten events with their payloads, and `tests/push.rs` checks each against the Rust build's Server-Sent Events, with the stream ending on a sign-out and on a password change elsewhere. Against 0.1.x, whose push was SignalR, those tests say they were skipped.
- **Skakel:** 0.1.x speaks Skakel protocol version 1 and the Rust build version 2; the tests take the version a target lists in `/skakel/info`. Pairing between two Koppels, the connection test outcomes, discovery and the token steps run against both. `KOPPEL_CONTRACT_SKAKEL=0` skips them for a build without Skakel, and `KOPPEL_CONTRACT_DISCOVERY=0` skips the tests that need the client to find the main target by broadcast. `DevicesChanged` is checked only on a target that can share a device.
- **Gaps:** what needs USB hardware (sharing, attaching), a second network, or a server older than Koppel kept secrets in is not covered; the crate's tests say so where it applies.

Against the Rust build, from the repo root (Git Bash on Windows):

```sh
bash src/koppel-contract/run-against-rust.sh              # all of them
bash src/koppel-contract/run-against-rust.sh --test push  # one file
```

The script builds `koppel`, starts the same six targets as processes, each on a fresh temporary data
folder, on seven ports, 16240 to 16246 (the guarded target takes two; `KOPPEL_CONTRACT_PORT` moves them), runs the tests and stops them. The
client target looks for servers on the main target's port, as the two 0.1.2 containers share one. On
Windows the targets listen on 127.0.0.1 only, with discovery and the USB/IP gateway off, so no firewall
prompt comes up and none takes port 3242 from an installed Koppel: the discovery tests and the
`Ui:AllowFrom` test are skipped there, since every caller is loopback. Elsewhere the client finds the
main target by broadcast and the script reaches the `Ui:AllowFrom` target at the machine's own address. `KOPPEL_CONTRACT_RELEASE=1` tests a release build.

Against Koppel 0.1.2, from the repo root with Docker running (Git Bash on Windows):

```sh
bash src/koppel-contract/run-against-0.1.2.sh            # all of them
bash src/koppel-contract/run-against-0.1.2.sh --test auth   # one file
```

The script starts six containers from `matumisoftware/koppel:0.1.2` on fresh volumes and a network of
their own, publishes them on 127.0.0.1 ports 15240 to 15246 (never 5240, where an installed Koppel
listens), runs the tests one at a time and removes everything it started, also when a test fails.
`KOPPEL_IMAGE` runs another image and `KOPPEL_CONTRACT_PORT` moves the ports.
