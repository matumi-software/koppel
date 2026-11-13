# Changelog

## 0.2.1 (2026-11-13)

Koppel 0.2.1 is the first release after the 0.2.0 review. It closes a hole in the UI's guards, keeps UI sessions in
memory, moves the machine-to-machine protocol to Skakel 3, versions the UI's REST API, and tidies how Koppel is
configured and how it keeps its secrets. What Koppel does for a person is the same: the same devices are shared and
attached the same way.

**Read this before you upgrade:**

- **Skakel 3 does not talk to 0.2.0.** A 0.2.1 machine and a 0.2.0 machine cannot link (each shows Skakel's version
  message), and neither can a 0.2.1 machine and a 0.1.x one. Update both ends of a link together. A client keeps its
  saved connection and links again once its server runs 0.2.1 too; nobody pairs again.
- **Everyone signs in again.** Sessions are kept in memory under a new cookie, so the first start of 0.2.1 ends every
  session, and so does every restart from now on.
- **Back up the data folder before the first start.** `secrets.json` is rewritten into a new shape at that start,
  and there is no downgrade: 0.2.0 cannot read the new file. Copy `/var/lib/koppel` (the `.deb`), the `koppel` volume
  (Docker) or `%ProgramData%\Koppel` (Windows) first if you may go back.
- **On Linux and in Docker, the first password needs a setup token.** When `KOPPEL_UI_ALLOW_FROM` is `lan` (the
  default) or `any`, Koppel prints a one-time token in its log, and the first-run screen asks for it:
  `docker compose logs koppel` or `sudo journalctl -u koppel`. An install that already has a password is not
  affected; after `reset-password`, setup needs a new token. Windows has none: the installer sets `local`.
- **The UI's REST API is under `/api/v1`, with renamed verbs.** The unversioned `/api/...` paths answer 404. Only
  the UI uses them, and the UI and the program move together, so this matters to a script of your own only.
- **Settings are `KOPPEL_*` variables and `--options`.** The 0.1.x names (`Ports__Ui`, `Paths__Config`, `--Ports:Ui`,
  `--reset-password` and the rest) still work and are logged once at Warning as deprecated. Rename them when it suits
  you; the table is in `docs/config.md`.

### Security

- **Fixed: the UI's access rules could be skipped.** A request that sent an `application/grpc+...` Content-Type was
  handed to the network side without the check of `KOPPEL_UI_ALLOW_FROM`, so on the default single port a caller
  outside the allowed networks could reach the UI's routes (the sign-in limit still applied, and protected routes
  still asked for a session and answered 401). The surface is now chosen from the path and the port only, never from
  a header. Update if the UI is reachable from a network you do not fully trust.
- Each route is protected by the router it sits on, not by a table of prefixes: a path no route has answers 401
  signed out and 404 signed in, paths are matched as written (`/API/settings` is the UI's page, not data), and a
  request that reaches a router without a caller address is refused. `KOPPEL_UI_ALLOW_FROM` now answers before the
  role gate, so a refused address sees 403 for every UI path where it used to see 404 for some.
- **First-visitor setup** is limited by the setup window (10 minutes, `KOPPEL_UI_SETUP_WINDOW_MINUTES`) and, on
  Linux and in Docker, by the setup token above. `GET /api/v1/auth/state` has a new field `setupTokenRequired`, and
  `POST /api/v1/auth/setup` takes an optional `setupToken` and answers 403 without the right one.
- **Sign-in is harder to guess at.** An IPv6 /64 counts as one caller for the lock, and a global budget of 30 tries
  in 10 minutes applies to all callers together; past it every caller gets 429 until the window is over. A locked
  administrator uses `reset-password`.
- **Sessions are in memory,** with an opaque 256-bit id in the cookie `__Host-koppel-session` (it was `koppel.ui`;
  the old cookie is ignored). A session lasts 12 hours without use and 7 days at most. `keys/ui-session.key` is
  deleted at the first start, and `ui-auth.json` keeps the password hash only (its `securityStamp` and
  `revokedSessions` are read and dropped).
- **Pairing only while a person has opened the window.** A server takes no pairing requests until an administrator
  presses Allow pairing on the Clients page (10 minutes, 1 to 60). Outside it a request gets 403.
- **Secrets stay out of logs and mostly out of memory.** The secret fields print as `Secret(<redacted>)`, and the
  values Koppel owns are wiped when dropped. This is best effort: see the threat model in `docs/security.md`.
- **The key can live outside the volume** (see Settings). On Linux and in Docker the default key still sits beside
  the files it protects. The threat model now says so, and says that the 0.1.x key ring left in `keys/` after the
  upgrade can still open whatever 0.1.x sealed.
- Every answer under `/api` carries `Cache-Control: no-store`, and the UI's `Content-Security-Policy` is a full
  policy (`default-src 'self'`, no framing) where it was only `frame-ancestors 'none'`. `X-Forwarded-Proto` no longer
  changes the origin check; the browser's `Sec-Fetch-Site` decides.
- At most 512 connections per listener are served at once, a UI request body that stalls for 10 seconds is cut, and
  Koppel stops at once with a UI tab open instead of waiting out the grace period.
- UI files carry an `ETag`, so a browser revalidates and gets 304.

### Wire and REST

- **Skakel protocol 3** (`/skakel/v3`, package `skakel.v3`, TLS 1.3 only, eight-digit pairing codes bound to the
  client name, fingerprints as bytes).
- **`/api/v1`.** The UI's REST API moved from `/api/...` to `/api/v1/...`; `/api/remote/*` (other Koppels) did not.
  Renamed or re-methoded endpoints:
  - `POST /server/clients/{id}/rotate` is `POST /server/clients/{id}/secret`
  - `POST /client/devices/{bus}/attach` is `PUT /client/devices/{bus}/attachment`, and `.../detach` is `DELETE` on it
  - `POST /server/pairing/window` is `PUT` (closing stays `DELETE`)
  - `POST /server/pairing/{id}/approve` is `POST /server/pairing/requests/{id}/approval`, and `.../{id}/deny` is
    `DELETE /server/pairing/requests/{id}`
  - `POST /client/pairing/confirm` is `POST /client/pairing/confirmation`
  - `POST /server/keys/next-certificate/switch` is `PUT /server/keys/current-certificate`
  - removed: `POST /server/keys/signing-key/rotate` (Skakel 3 tokens are not signed)
- **Refusals are problem JSON** (`application/problem+json`, RFC 9457) with a stable `code`. The sign-in endpoints'
  `{ "error": "..." }` is now `detail`, a wrong or weak password in the password change is a validation problem on
  `current` or `next`, and the origin check (403), the sign-in limit (429, now with `Retry-After: 60`), a missing
  session (401), an unknown API path (404), an unreadable body (400) and a wrong media type (415) have bodies where
  they had none. A failed approval save answers 500, not 404. JSON answers are `application/json` without
  `; charset=utf-8`, dates are written with `Z` instead of `+00:00`, and request bodies are read with camelCase names.
- **Pairing window:** `PUT` and `DELETE /api/v1/server/pairing/window`; `window` is added to the pairing overview and
  to `PairingRequestsChanged`. The keys view and `ServerKeysChanged` no longer carry `signingKeyId`,
  `signingKeySince` or `retiredKeysUntil`.
- **The live push opens with the state** of each resource the machine's role has, as the same events under the same
  names, so the UI fetches only what the snapshot does not cover after a reconnect. No new event name or payload.
- **New:** `GET /readyz` (the gateway, the certificate and the link to the server, as states only), and
  `/api/v1/system/info` gains `commit` and `built`.
- A server also answers discovery probes over IPv6 now, on the same UDP port, on a socket of its own.

### Upgrade and on-disk

- **`secrets.json` is rewritten once, at the first start** of 0.2.1, from 0.1.x's or 0.2.0's shape into Skakel's own
  state: the certificate as DER and a PKCS#8 key (no PKCS#12), no token signing keys, no null keys. The DER is the
  same, so pinned clients keep their pin, and the clients, the next certificate and the connection carry over.
  `koppel-upgrade` does this, and the 0.1.x Data Protection conversion, before any file is opened. Other files are
  read as they are and take the new shape when next written. **There is no downgrade.**
- Every config file written by 0.2.1 ends with `"version": 1`, which older builds ignore; a file without it is read
  as version 1, and a file with a newer version stops the start and is left alone. Writes are flushed, replaced and
  the directory flushed, so a power cut cannot bring an old file back. Temp files are `0600`.
- A `settings.json` that leaves out `shareOnStartup` now means true (it was false).
- On Linux a symlinked or foreign-owned config or keys directory is refused at start.
- `keys/ui-session.key` is deleted. **The 0.1.x key ring (`key-*.xml` in `keys/`) is left where it is.** Koppel never
  deletes it; once you have a backup, delete the files by hand. There is no command.
- New on Windows: `%ProgramData%\Koppel\logs\koppel.log` (rotated at 5 MiB, five files, administrators only), and the
  service has its own SID, `NT SERVICE\Koppel`.
- New command: `koppel diagnostics [file]` writes one report to attach to a support request (the encrypted files and
  keys are never read, and values named like a secret are removed).

### Settings

- **`KOPPEL_*` settings and `--option` forms for every setting,** built once at start (`koppel --help` lists them),
  plus the commands `serve` (the default), `reset-password` and `healthcheck`, and `koppel --version` with the commit
  and the build date.
- **Deprecated, still working, logged once:** `Ports__*`, `Paths__*`, `Ui__*`, `Usb__*`, `Gateway__*`, `Discovery__*`,
  `RateLimits__*`, `--Section:Key`, `ASPNETCORE_ENVIRONMENT`, `--reset-password` and `--healthcheck`. When both names
  are set, the new one wins. A compose file, `/etc/default/koppel` or a service registry value that uses the old
  names needs no change to keep working; the compose files and installers here use the new ones.
- **Stricter:** an unknown command line option or an invalid value stops the start (exit 2) and names the option,
  where it used to be skipped. An unknown `KOPPEL_` or `Section__Key` variable is warned about, with the nearest known
  name.
- **New:** `KOPPEL_KEY_FILE` (Linux and Docker), a systemd credential named `koppel-key` and a Docker secret named
  `koppel-key` keep the key that seals the secret files outside the volume, tried in that order. To move an install
  over, set the source, start, wait for the log line that the files were sealed again, then move `koppel-secrets.key`
  out of the keys directory, and keep the outside key: **the files cannot be read without it.**
  `koppel-secrets.<anything>.key` files in `keys/` are read as older keys. The `.deb`'s unit and `docker-compose.yml`
  carry the commented examples.
- `Paths:KeyCertificate` has not been read since 0.2.0; setting it logs a warning.

### Packaging and installers

- The image's health check runs `koppel healthcheck`, and the image sets `KOPPEL_CONFIG_DIR` and `KOPPEL_KEYS_DIR`.
  The Docker composes limit the container's log to three files of 10 MB.
- The Windows installer sets `KOPPEL_UI_ALLOW_FROM=local` for the service (it was `Ui__AllowFrom`), and an upgrade
  keeps the service's state.
- The `.deb` no longer enables a unit the administrator disabled, and no longer restarts a stopped service on upgrade.
- New: `koppel_<version>_{amd64,arm64,armhf}.tar.gz` beside the `.deb` files, for Linux distributions that do not
  install a `.deb`. It holds the static binary, the unit, the man page and an `install.sh` that sets up the service
  the way the `.deb` does; `uninstall.sh` removes it. The README says how.
- The embedded UI is about 240 KB smaller (Latin fonts only: Cyrillic, Greek or Vietnamese text in a monospace field
  now uses the system font). The UI recovers by itself after a first connect that failed.
- Navigation links open in a new tab, each page sets its title and takes focus on its heading, there is a skip link
  and one screen reader voice, and the password forms say what to fix.
- A tray line with a known type and a field of the wrong kind is skipped whole.

### Developer-facing

- There is no CI and no bot: no workflow runs on a hosted runner, Dependabot is gone, and the release is a checklist
  run by hand ("To release" in `docs/building.md`). Run `cargo fmt --all --check`, `cargo build`, `cargo clippy` and
  `cargo deny check` with `--locked`, the tests in a Linux container, and `.github/scripts/check-pins.sh`.
  `rustup` reads `src/rust-toolchain.toml` and installs Rust 1.99.1 on the first cargo run from `src/`.
- **Crate layout:** `koppel` (wiring) over `koppel-windows` and `koppel-web` over `koppel-app` over `koppel-usb` over
  `usbip`, with `koppel-config`, `koppel-secrets` and `koppel-upgrade` beside them. `koppel-app` is new (the roles,
  the activity log, the push bus, the tray rules), `usbip` is new (the USB/IP protocol with no Koppel in it),
  `koppel-secrets` is new (`Secret<T>`), and `koppel-usb` names no config file. `koppel-windows` is out of the
  default members. None of these crates is published.
- The wire types of the UI are generated from the Rust types: after changing a type the API sends or reads, run
  `KOPPEL_WRITE_WIRE=1 cargo test -p koppel-web wire` and commit `generated/wire.ts`. New dependency: ts-rs 12.0.1
  (MIT). A connection test whose request itself failed reports `unexpectedAnswer` in the UI.
- The contract tests run in the cargo workspace, each starting `koppel` on a free port (no scripts, no Docker).
- Public API changes inside the workspace: `UsbClient::new` and `UsbServer::new` take `Deps` and `ServerDeps`;
  koppel-config's `SecretBox` gained `with_sources`, `source`, `keys_read` and `reseal`, and the secret fields of its
  file structs became `Secret<String>`.

### Not tested on real hardware or by hand

What the tests cover is listed in the pull requests. A person has not run these on the real thing:

- Real USB hardware with 0.2.1. Only a Debian 13 server in Docker sharing to a native Windows 11 client has run on
  hardware, with 0.2.0 (2026-11-09). A Linux client, a Windows server through usbipd-win and a Raspberry Pi are open
  under #115.
- The Windows installer: install, upgrade from 0.2.0, uninstall, the tray, the new log file and the service SID
  (`NT SERVICE\Koppel`).
- systemd credentials (`LoadCredential=koppel-key:...`) with the `.deb`'s unit, and the key outside the volume with a
  Docker secret.
- IPv6 multicast discovery (`ff02::1`) between two machines. The tests use unicast probes to `::1`.
- The Debian package: install, upgrade from 0.2.0 (the new unit rules) and purge.
- The setup token on a real Linux or Docker first run through a browser, and the 0.2.0 to 0.2.1 `secrets.json`
  upgrade on a real install (it is tested on fixtures of 0.1.x and 0.2.0 files).

## 0.2.0 (2026-11-09)

Koppel 0.2.0 is a rewrite in Rust, with the same web UI and features, small enough for a Raspberry Pi Zero.
Breaking changes:

- `Paths:KeyCertificate` (`Paths__KeyCertificate` in a compose file) is deprecated and goes in 0.2.0. A Linux install
  or container that sets it cannot carry its encrypted files over: after the upgrade the server's clients pair again,
  and a client pairs again with its server. Installs without it, which includes every Windows install, keep their
  pairings. Do not start using it.
- There is no way back from 0.2.0 to 0.1.x: the Skakel protocol changes too, so a 0.2 data folder is not for 0.1.x.
- The UI's live updates use Server-Sent Events at `/api/events` instead of SignalR at `/api/hubs/ui`.
- Koppel machines talk Skakel protocol version 2 (gRPC), so a 0.2.0 client and a 0.1.x server, or the other way round,
  cannot link: update both. Pairings carry over.
- A server whose `Ports:Address` is an IPv4 address answers discovery on that address only, so one that listens on
  loopback no longer answers the local network.

Linux packages and the image:

- The `.deb` comes for amd64, arm64 and armhf, the last for 32-bit Raspberry Pi OS on any Pi, the Zero
  included. Each is about 3.5 MB instead of about 50 MB, and needs no other package: no OpenSSL, no ICU. It
  upgrades 0.1.x in place and keeps `/var/lib/koppel`.
- The `.deb` adds Koppel to the applications menu. It opens the UI in your browser, on the port
  `/etc/default/koppel` sets.
- systemd restarts a Koppel that hangs, not only one that exits (`WatchdogSec=30`).
- The service runs with systemd's sandbox: the disk read-only apart from `/var/lib/koppel`, no
  capabilities but binding a low port, and `/dev` limited to USB devices. `systemd-analyze security koppel`
  scores 2.1 instead of 9.4.
- The image is built on distroless (no shell) for amd64, arm64 and arm/v6, about 4.5 MB instead of about
  100 MB. Its health check is `koppel --healthcheck`, as in 0.1.x.
- The `.deb` and the image carry the third-party notices at `/usr/share/doc/koppel/THIRD-PARTY-NOTICES.txt`, and
  the `.deb` a Debian copyright file beside them.

Other changes:

- The Koppel mark in the UI draws the K in the text colour and keeps only its circle in canopy green, part of the
  Matumi rebrand. The app icon does not change.
- After a wrong password, the sign-in form sent nothing more until the page was reloaded, even with the right
  password typed in. The browser held the field's error as a validation error and refused to submit the form. Every
  form in the UI had the same problem after a refusal from the server.
- The C# code is gone from the repo; the 0.1.x source stays at tag `v0.1.2`.

## 0.1.2 (2026-11-09)

- The Windows app has a Koppel icon in the notification area for every signed-in user, the way Apollo does it. Click it
  to open the UI in your browser. Its menu also shuts Koppel down, after asking first; Koppel then stays off until the
  next boot or until the service is started again. The icon comes and goes with the service.
- The tray icon's tooltip says how Koppel is doing: what a client is linked to and how many devices it attached, or
  what a server shares and how many are in use. An amber dot on the icon means the link is down, sharing is not
  available, or setup is waiting.
- The tray icon shows Windows notifications: a client waiting to pair (a click opens Clients), the link to the server
  dropping and coming back, once each, and a device that does not attach. Windows holds them back during Do not
  disturb. Each kind can be turned off under Configuration, Notifications, in the Windows app.
- **Koppel** in the Start menu opens the UI. If Koppel is not running, it offers to start it (an administrator has to
  approve), waits until the UI answers and then opens it. The tray's shut down dialog points to it.
- The tray icon's menu has **Restart Koppel**, which any signed-in user can use, like Shut down. When setup has timed
  out, the UI points to it instead of an elevated prompt.
- The last page of the Windows installer has **Launch**, which opens the UI to set the first password. It shows after
  an install or an upgrade, not after a repair or an uninstall.

## 0.1.1 (2026-11-06)

Koppel 0.1.0 and 0.1.1 cannot talk to each other. Update the server and its clients together.

- Koppel's transport moved to [Skakel](https://github.com/matumi-software/skakel), a library under the MIT
  licence: finding servers, pairing, tokens, certificate pinning and the certificate handover, the link to
  the server, and the stream that carries USB traffic. Koppel keeps USB, its config files and the UI.
- Skakel speaks protocol version 1, a clean break from what 0.1.0 spoke: new paths under `/skakel`, new
  discovery packets, and a version check before anything else. Its wire format is in Skakel's
  [`docs/protocol.md`](https://github.com/matumi-software/skakel/blob/main/docs/protocol.md).
- A server keeps its certificate, its signing key and its clients, and a client keeps its connection, so a
  pairing made with 0.1.0 works again once both ends run 0.1.1. A client added before Koppel kept secrets
  in their current form is dropped, says so in the log, and pairs again. Every client pairs again if the
  server cannot carry its 0.1.0 secrets over, which the log says too.
- A client that was revoked, or whose secret was rotated, now says it has to pair again.
- The connection test can also answer `versionMismatch`, naming both ends' protocol versions,
  `unexpectedAnswer` and `tooManyRequests`.
- Skakel 0.1.0 brings its own hardening: token and pairing requests are limited per address and capped at
  4 KiB before they are read, a client gets 429 after 10 wrong secrets in a minute, client names lose
  control and format characters, a client reads at most 64 KB of any answer, and a machine that copies a
  server's fingerprint no longer hides the real one. Fewer packages ship: OpenIddict's client and Polly
  are gone.
- The servers on your network say when other addresses answer with the same fingerprint.
- The foot of the sidebar is one status card: the link, the machine name, what a client is linked to, Sign
  out, and the running Koppel version.
- A probe the server could not answer, such as one from UDP port 0, stopped discovery until a restart. Now
  only that answer is skipped.
- The UI's sign-in lockout and rate limit counted an IPv6 /64 as one caller, so one host on a network could
  lock every IPv6 host on it out of the UI. They now count each whole address.
- Koppel is now licensed under GPL-3.0-or-later. 0.1.0 and earlier stay under the MIT licence they were
  released with.

## 0.1.0 (2026-11-03)

The first release. Koppel shares USB devices over the network: a server shares the devices plugged into it,
and a client attaches them as if they were plugged in locally. It speaks the standard USB/IP protocol and
adds a web UI, sign-in and encrypted transport around it.

### Platforms

- Linux server and client, in Docker (`matumisoftware/koppel:0.1.0`, amd64 and arm64) or as a `.deb` that
  runs Koppel as a systemd service on Debian 12 and 13, Ubuntu 22.04 and later, and 64-bit Raspberry Pi OS.
- Windows client from `Koppel-Setup.exe`, which installs Koppel as a service with usbip-win2's signed driver.
- Windows server through usbipd-win (`Usb:Upstream`), natively or in Docker Desktop. See `docs/windows.md`.

### Finding, pairing and the UI

- Clients find servers on the local network with a small UDP probe. An answer is never larger than its
  probe, so a server cannot be used to amplify traffic.
- A client pairs by showing a six-digit code that you approve on the server. The client gets its own
  credentials, so nobody copies secrets around. A server on another network is entered by address.
- The UI shows devices, clients, activity and settings. A change shows as pending until the server has
  written it and confirms it, on every open browser.
- Everything uses one port, 5240: the UI, tokens, pairing, the remote API and USB traffic on TCP, and
  finding servers on UDP. `Ports:Network`, `Discovery:Port` and `Ports:Address` move or bind them.

### USB

- A server exports only a device you shared, and only while it is the same device on that port.
- An attach that stalls fails after 20 seconds (`Usb:AttachTimeoutSeconds`) with a readable reason.
- USB sessions ping both ways, so a vanished peer frees the device in under a minute, and a client
  attaches its remembered devices again after a restart or a network outage.
- On native Windows the local gateway for usbip-win2's driver listens on 127.0.0.1 and serves only the
  kernel driver's own imports. Stopping Koppel detaches the devices it attached.

### Security

- The UI needs a password. The first person to open the UI sets it, within 10 minutes of Koppel starting
  (`Ui:SetupWindowMinutes`); after that the UI says setup has timed out and to restart Koppel.
  `Ui:AllowFrom` limits who can reach it. `Koppel --reset-password` clears a forgotten one and opens setup again.
  Wrong tries are slowed down. Changing or resetting the password ends every session, and signing out ends
  that session for good, also after a restart.
- The UI is served over HTTPS with the server's own certificate; `http://` is redirected. The UI answers
  only the local network by default (`Ui:AllowFrom`: `local`, `lan` or `any`), and the Windows installer
  sets `local`. Other websites cannot drive it (Origin check, a required `X-Koppel-UI` header, a SameSite
  cookie), and responses carry security headers.
- Clients pin the server's certificate and authenticate with their own ID and secret. Revoking a client,
  or rotating its secret, ends its running USB sessions at once.
- The token signing key and the server certificate can be rotated from the server's Clients page. Paired
  clients learn the new certificate before the switch.
- Pairing, the token endpoint and sign-in are rate limited, and discovery results are size-limited and
  expire.
- Secrets are kept in encrypted files. The config and keys folders are locked to administrators on Windows
  and to root (`chmod 700`) on Linux. On Windows the key ring is protected at rest; on Linux only by the
  folder, unless `Paths:KeyCertificate` points at a certificate kept outside the data volume.

### Reliability

- An unreadable or invalid config file stops Koppel with a message naming the file; it is never replaced
  with defaults. A bad hand edit of `settings.json` while Koppel runs is ignored, with a system check that
  says why. Files are written atomically.
- The Windows service restarts after a crash and after it stops with an error. The `.deb` service and the
  container restart too, and the image has a health check on `/healthz`.

### Installer and deploy

- The Windows installer adds inbound firewall rules for Koppel on Private and Domain networks only, and
  locks `%ProgramData%\Koppel` to administrators. It refuses a data folder owned by someone who is not an
  administrator, with instructions. A usbip-win2 you installed yourself is used and left alone, and
  uninstall removes only what Koppel installed. An upgrade keeps the driver, the config and the keys.
- The `.deb` loads the USB/IP modules at boot, keeps `/var/lib/koppel` on upgrade and remove, and deletes
  it on purge. Settings go in `/etc/default/koppel`.
- The Docker compose files pin `matumisoftware/koppel:0.1.0`. `prepare-host.sh` installs Docker and loads
  the USB/IP modules.
- Builds are pinned: base images, package lock files, and the vendored usbip-win2 files with their hashes
  and signers. `THIRD-PARTY-NOTICES.txt` ships with the installer, the package and the image.

### Tested

On real hardware (2026-11-03): a Debian 13 server shares two Logitech receivers and a flash drive to a
Windows 11 client, natively and through Docker Desktop. A 256 MB file on the drive keeps its checksum
through a detach and reattach, and a 75 second network drop is recovered from by itself. A Windows server
through usbipd-win shares a webcam to a Debian client at 30 fps. The `.deb` installs, upgrades, removes and
purges on Debian 13, and the Windows installer installs, upgrades and uninstalls on a clean Windows 11 VM.

### Known limits

- Warning: the arm64 image and the arm64 `.deb` are untested on a Raspberry Pi. Do not rely on them yet.
- Printers are untried. Storage works.
- Webcams do not stream from a Linux server to a Windows client (a Logitech C270 attaches but delivers no
  frames, #22). The other direction works. If another program on a Windows server holds the camera open,
  usbipd-win cannot export it until that program lets go.
- A client connects to one server at a time (#21).
- The UI's certificate is self-signed. Accepting the browser's warning on a network you do not trust could
  let an impostor collect the password; the fingerprint under Clients is the one to compare.
- The Linux container is privileged and runs as root. The Windows service runs as LocalSystem.
- The installer is not code signed, and there is no CI.
- The rollback of a failed fresh install was not exercised. The attach deadline was tested with fakes only.
- There is no apt repository yet, so the `.deb` is installed and upgraded from a file.
