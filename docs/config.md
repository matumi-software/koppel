# Config

## Settings

Koppel's settings are one typed struct (`koppel/src/config.rs`), built once at start by clap. Each is a
command line option or an environment variable, and the option wins. The name of the variable is
`KOPPEL_` and the option's name in capitals with `_` between words (`--ui-port` and `KOPPEL_UI_PORT`).
A blank value counts as not set. `koppel --help` lists them with their variables.

```sh
koppel [options] [serve]           # run Koppel; no command means serve
koppel [options] reset-password    # clear the UI password and end every session
koppel [options] healthcheck       # ask a running Koppel's /healthz: exit 0 if it answers, 1 if not
koppel --version                   # koppel 0.2.0 (commit 1a2b3c4d5, built 2026-11-12)
```

A value that is not valid, or an option Koppel does not have, stops the start before anything runs, with
exit code 2 and a message that names the option. `--version` gets its commit from the build (the image
build passes it in, since it has no `.git`) and says `unknown` when it has none.

| Option | Variable | 0.1.x name | Default |
|---|---|---|---|
| `--ui-port` | `KOPPEL_UI_PORT` | `Ports:Ui` | 5240 |
| `--network-port` | `KOPPEL_NETWORK_PORT` | `Ports:Network` | the UI port |
| `--listen-address` | `KOPPEL_LISTEN_ADDRESS` | `Ports:Address` | every address |
| `--ui-allow-from` | `KOPPEL_UI_ALLOW_FROM` | `Ui:AllowFrom` | `lan`, `local` in development |
| `--ui-setup-window-minutes` | `KOPPEL_UI_SETUP_WINDOW_MINUTES` | `Ui:SetupWindowMinutes` | 10 |
| `--config-dir` | `KOPPEL_CONFIG_DIR` | `Paths:Config` | the platform's (see below) |
| `--keys-dir` | `KOPPEL_KEYS_DIR` | `Paths:Keys` | the platform's |
| (none) | (none) | `Paths:KeyCertificate` | not read since 0.2.0; setting it logs a warning |
| `--rate-auth-per-minute` | `KOPPEL_RATE_AUTH_PER_MINUTE` | `RateLimits:AuthPerMinute` | 30 |
| `--rate-token-per-minute` | `KOPPEL_RATE_TOKEN_PER_MINUTE` | `RateLimits:TokenPerMinute` | 60 |
| `--rate-pair-per-minute` | `KOPPEL_RATE_PAIR_PER_MINUTE` | `RateLimits:PairPerMinute` | 120 |
| `--discovery-enabled` | `KOPPEL_DISCOVERY_ENABLED` | `Discovery:Enabled` | true |
| `--discovery-port` | `KOPPEL_DISCOVERY_PORT` | `Discovery:Port` | the network port |
| `--usb-sys-root` | `KOPPEL_USB_SYS_ROOT` | `Usb:SysRoot` | `/sys` |
| `--usb-upstream` | `KOPPEL_USB_UPSTREAM` | `Usb:Upstream` | none |
| `--usb-usbip-path` | `KOPPEL_USB_USBIP_PATH` | `Usb:UsbipPath` | the bundled copy, then a standalone install |
| `--usb-attach-timeout-seconds` | `KOPPEL_USB_ATTACH_TIMEOUT_SECONDS` | `Usb:AttachTimeoutSeconds` | 20 |
| `--gateway-port` | `KOPPEL_GATEWAY_PORT` | `Gateway:Port` | 3242 on native Windows, off elsewhere; 0 turns it off |
| `--gateway-address` | `KOPPEL_GATEWAY_ADDRESS` | `Gateway:Address` | `127.0.0.1` |
| `--gateway-max-sessions` | `KOPPEL_GATEWAY_MAX_SESSIONS` | `Gateway:MaxSessions` | 64 |
| `--environment` | `KOPPEL_ENVIRONMENT` | `ASPNETCORE_ENVIRONMENT`, `DOTNET_ENVIRONMENT` | none; `development` makes the UI `local` |

Log levels come from `RUST_LOG`, not a setting.

**0.1.x's names still work in 0.2.1**, and each one that is used is logged once at Warning as deprecated,
with the name to use instead. That covers the environment (`Ports__Ui`, `Paths__Config`, `Ui__AllowFrom`),
matched without regard to case; the command line (`--Ports:Ui=5250`, `--Ports:Ui 5250`, `/Ports:Ui=5250`),
which wins over the environment as before; and the flags `--reset-password` and `--healthcheck`, which are
the commands `reset-password` and `healthcheck`. When both the new variable and the old one are set, the
new one wins and the log says the old one is ignored. A compose file, `/etc/default/koppel` or a service
registry value that uses the old names needs no change to keep working, and the installers and compose files in
this repository use the new names.

**A name Koppel does not know is warned about, never silently ignored.** Any `KOPPEL_` variable that is not
in the table, and any `Section__Key` variable that is not one of 0.1.x's, is logged at Warning with the
nearest known name when there is one (`Ports__Uii is not a Koppel setting and is ignored. Did you mean
Ports__Ui?`). `KOPPEL_RUNNING_IN_CONTAINER`, `KOPPEL_UI_URL`, `KOPPEL_HOSTNAME`, `KOPPEL_IMAGE` and
`KOPPEL_CONTRACT_*` are read by other parts of Koppel and its tools, and are not warned about.

## Config on disk

The config directory holds:

- `settings.json`: plain, may be edited by hand.
- `secrets.json`: Skakel's server state (`SkakelState`): the HTTPS certificate, the registered clients and the prepared next certificate. Encrypted. Created on every machine at first start, whatever the role, because the certificate is made then. A file from Koppel 0.2.0 still holds a token signing key; Skakel 0.3.0 does not sign tokens, so it is dropped when the server starts. Skakel reads it once at start and writes it after every change (`koppel-web/src/roles/skakel/store.rs`). A file Koppel 0.1.0 wrote is carried over on the first start: the certificate and the clients whose secret hash Skakel reads (a client added before Koppel kept secrets in their current form is dropped, and logged). A 0.1.0 file that cannot be carried over is copied aside as `secrets.json.0.1.0-<UTC time>` and the server starts with a new certificate and no clients, which the log says.
- `connection.json` (client role): server address, pinned fingerprint, client ID and secret, and during a certificate handover the server's next fingerprint. Encrypted. Its format did not change in 0.1.1, so a client keeps its connection, and once its server runs 0.1.1 too it links as before.
- `ui-auth.json`: the UI password hash and security stamp. Encrypted.
- `devices.json` (server role): the bus IDs the user chose to share, with their VID:PID. Plain.
- `attached.json` (client role): the devices this client attached, with the server they belong to. The client attaches them again whenever the server offers them and nobody is using them: after a restart, an upgrade, a dropped link, or a replug on the server. Detaching forgets the device. Plain.

How a file is read and written:

- **Start-up never replaces a file.** Only a missing file is seeded with defaults. A file that exists but cannot be used (unreadable after a few retries, cannot be decrypted, is not valid JSON, or fails validation) stops start-up with an error that names the file, the cause and the likely fix, and the file is left exactly as it was.
- **Two plain files start with defaults instead.** `devices.json` and `attached.json` are not critical and their defaults are safe (nothing shared, nothing remembered), so a corrupt one does not stop Koppel. It starts empty in memory, the file is left untouched, and `GET /api/system/checks` shows a `devices` or `attached` entry ("devices.json was not used.") with the reason. The first change made while the file is bad (sharing a device, attaching one) moves it aside as `<name>.unreadable-<UTC timestamp>`, logs a warning, and writes a fresh file; fixing the file by hand instead clears the entry. `settings.json` and the encrypted files still refuse to start: defaults would put a machine back in the Server role, or lose keys and credentials.
- **Hand edits are validated** the same way a REST change is, including the port rules. While Koppel runs, a bad edit is ignored: the last good value stays, a warning is logged, and `GET /api/system/checks` shows a `settings` entry, "settings.json was not applied", with the reason. A later valid edit clears it. A REST change made over a bad hand edit replaces the bad file with the last good value plus the change.
- **A valid hand edit** is loaded before the next change, so a change from the UI never overwrites it.
- **Pushes are ordered.** A change reaches subscribers in write order, one change at a time, even when several writes are in flight. A handler may update the same file: the nested change is written and applied at once, its delivery is queued behind the running one and anything already waiting, so every subscriber sees the same order, and the nested call returns without waiting for that delivery (it would wait for itself). A handler that changes the file from another task and waits for it is not seen as nested, so that call gives up waiting for the delivery after 15 seconds and returns; its change is on disk by then.
- **Writes** go to a random temp name (mode `0600` on Linux), are flushed, and replace the file, and then the directory is flushed too, so a power cut cannot bring the old file back after the change was acknowledged. A failed write removes its temp file, the replace is retried on a sharing error, and stale temp files are swept at start. Windows has no directory flush; NTFS journals its metadata.
- **Own writes are told from edits by content.** After a write the file's size and time are taken from the temp file before it is moved, and a SHA-256 of the bytes is kept. A file whose size or time differs is read and hashed, and one with the same hash is our own write. A file written in the last three seconds is hashed on every poll, so an edit that keeps the size and a coarse file system time (`"server"` and `"client"` are both six letters) is still seen. No plaintext copy of a file is kept to compare with.
- **Limits and versions:** a config file over 8 MiB is refused without being read whole. Every file carries `"version": 1` after its fields, which 0.1.x ignores. A file with a newer version stops start-up (or is not applied, for a hand edit) and is left as it was, as a secret file with a newer format is. Names in a hand-edited file that Koppel does not know are logged at start and on each edit: they are ignored and are gone after the next change Koppel writes. A file without `version` is read as version 1.
- **Errors have kinds** (`ConfigErrorKind`: `Io`, `Decrypt`, `Parse`, `Invalid`, `Newer`), and the advice is composed from the kind: only a file that did not decrypt is told to restore the keys directory, and a full disk is not.
- **`shareOnStartup` has one default, true,** for a new install and for a hand-edited `settings.json` that leaves it out. 0.1.x read a missing one as false.
- **Enums are names only:** a number where a name is expected (`"role": 1`) is refused.

The encrypted files are sealed with a key in a separate keys directory (`KOPPEL_KEYS_DIR`): machine DPAPI on
Windows, and on Linux AES-256-GCM with a key file that is plain on disk (see "Secret files" in "Config
files in Rust"). 0.1.x used a Data Protection key ring there, which Koppel converts from once (see "The
upgrade from 0.1.x" there). `Paths:KeyCertificate`, which protected that key ring on Linux, is deprecated
and goes in 0.2.0, which cannot read a key ring protected with it.

Data folder hardening. Before anything is written, Koppel restricts the config and keys directories:

- **Windows:** the owner becomes Administrators and the ACL is protected (no inherited access) with full control for SYSTEM and Administrators only. A directory that is a junction or a symbolic link is refused. A process that is not elevated, such as a developer running from a console, cannot take ownership, so it also keeps its own access.
- **Linux:** a symbolic link is refused, as on Windows, and so is a directory owned by anyone but the user Koppel runs as (its owner could swap what is in it). Then `chmod 700`. A bind-mounted config directory that the process cannot chmod makes start-up fail.

Where the config directory is:

- Docker: `/koppel/config` and `/koppel/keys` inside the `koppel` volume.
- Otherwise: `%ProgramData%\Koppel\config` on Windows, `/var/lib/koppel/config` elsewhere, unless `KOPPEL_CONFIG_DIR` is set. A developer running `cargo run -p koppel` sets `KOPPEL_CONFIG_DIR` and `KOPPEL_KEYS_DIR` to a folder of their own.

### If the key ring is lost

The encrypted files can be read only with the key ring that wrote them. If the keys directory is
missing, replaced or restored from another machine, Koppel refuses to start and says which file it
cannot use ("Koppel cannot use ...secrets.json: ... The file was not changed ... restore the keys
directory"). It does not make new secrets over the old ones.

- **What you see:** in Docker the container exits and restarts, over and over (`restart: always`), with that message in `docker logs`, until the keys are back. On Windows the service fails to start and the recovery rule retries it; run `Koppel.exe` from an elevated prompt with the service stopped to read the message.
- **What to do:** restore the keys directory from a backup (the keys directory and the config directory belong together). If the keys are really gone, the encrypted files cannot be recovered: move the unreadable ones aside (`secrets.json`, `connection.json`, `ui-auth.json`) and start again. A new `secrets.json` means a new certificate, so every client must be paired again. A lost `connection.json` means entering or pairing the connection again. A lost `ui-auth.json` means setting the password again, within the setup window after the start.

### Config files in Rust

`koppel-config` reads and writes the same files in the same folders as 0.1.x (["Config on disk"](#config-on-disk)),
with the same rules.

- **JSON:** what 0.1.x wrote, byte for byte: two space indents, the
  platform's line ending, `\uXXXX` for everything outside printable ASCII and for `" & ' + < >` and the
  backtick, dates as `2026-11-09T08:30:15.1234567+00:00`, and the computed properties 0.1.x wrote
  (`isComplete`, `hasPassword`) written too. Names are read in any case and unknown ones ignored, a
  missing one takes its default, and an enum is a name in any case, never a number. The tests read
  files 0.1.2 wrote and write them back unchanged.
- **Writes and pushes:** `ConfigFile`: temp file, flush, replace with retries,
  stale temp files swept, start-up never replacing a file, `devices.json` and `attached.json` starting
  empty and moving a bad file aside, and changes delivered in write order with a nested change queued.
  Hand edits are found by checking the file's size and time every quarter second, which also works
  through Docker bind mounts, where file change events may not arrive.
- **Hardening:** as 0.1.x on Windows (owner Administrators, a protected ACL for SYSTEM and
  Administrators, an unelevated process keeping its own access, links refused, `%ProgramData%\Koppel`
  itself when the folders are in it), and on Linux a link refused, the owner checked and `chmod 700`.
- **Session key:** `ui-session.key` in the keys directory, sealed the same way, signs the UI's session cookies (see ["The UI's server side in Rust"](ui.md#the-uis-server-side-in-rust)).
- **Secret files:** a six byte header, `KPSF`, a format version (1) and a scheme. Scheme 1, on Windows,
  is machine DPAPI over the JSON. Scheme 2, elsewhere, is AES-256-GCM with a random key in
  `koppel-secrets.key` in the keys directory, mode `0600`; the file then holds that key's id and a
  nonce before the ciphertext. The key file is made through a temp file and a hard link, so it never
  exists half written; on a file system without hard links (some network, FUSE and Docker bind
  mounts) it is made in place with `create_new`, and a start that meets it mid-write waits for it.
  Two Koppels starting together end up with one key either way. Data Protection's key ring is no longer used. By default anyone who can
  read the keys directory on Linux can decrypt the files, since the key sits beside them; a key from
  `KOPPEL_KEY_FILE`, a systemd credential or a Docker secret moves it out (see "Keeping the key outside
  the volume" in `docs/security.md`).
- **The key ring:** the box reads every key it can find and seals with one. A key from outside the
  volume seals when there is one (and `koppel-secrets.key` in the keys directory is then only read,
  never made); otherwise `koppel-secrets.key` seals. `koppel-secrets.<anything>.key` files are older
  keys, only read. A file's key id says which key opens it, and at every start the secret files
  (and `ui-session.key`) that an older key sealed are written again with the current one, so a
  rotation, or a move of the key out of the volume, is one start. A key from outside is any secret
  of 32 to 4096 bytes; the AES key and the id come from it by HKDF-SHA256.
- **Secrets in memory:** the secret fields of the file structs are `Secret<String>`, which prints
  `Secret(<redacted>)`, wipes itself when dropped and reads and writes as the string it holds, so the
  files do not change. The plaintext of a file being read or written is wiped after use. No copy of a
  file's JSON is kept to see whether it changed (a SHA-256 of the bytes and a typed compare do that).
- **The upgrade from 0.1.x:** at the first start, each of `secrets.json`, `connection.json` and
  `ui-auth.json` that is still a Data Protection payload is decrypted with the 0.1.x key ring (machine
  DPAPI on Windows, plain XML on Linux) and written again in the new format, with the same JSON inside.
  All are decrypted before any is written, so a failure changes nothing: the log says why, and the
  start then stops on the first file still in the old format. The key ring is left as it was. A key
  ring protected with `Paths:KeyCertificate` is not read: its files are renamed to
  `<name>.0.1-key-certificate-<UTC time>`, unchanged, and Koppel starts as a new install, so every
  client pairs again and the password is set again.
- **What carries over:** the certificate, so pinned clients keep their pin; the clients and their
  secrets; the client's connection; the UI password. The token signing key does not carry over: Skakel 0.3.0's
  tokens are random strings the server remembers, tokens last 30 minutes and live only in a client's
  memory, and a 0.2 client cannot use a 0.1.x server's tokens anyway (the protocols differ). UI sessions end once: 0.1.x's
  cookies were protected by keys in `keys/aspnet`, which 0.2 does not read.
- **Certificate:** 0.1.x kept it as a password-less PKCS#12 (3DES and SHA-1, where "no
  password" is an empty BMPString without its terminator). The PKCS#12 crates refuse that form, so
  `koppel-config` reads it itself and hands the DER certificate and PKCS#8 key to rustls.
