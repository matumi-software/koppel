# Config

## Config on disk

The config directory holds:

- `settings.json`: plain, may be edited by hand.
- `secrets.json`: Skakel's server state (`SkakelState`): signing key, HTTPS certificate, registered clients, and during a rotation the keys being retired and the prepared next certificate. Encrypted. Created on every machine at first start, whatever the role, because the certificate and the signing key are made then. Skakel reads it once at start and writes it after every change (`koppel-web/src/roles/skakel/store.rs`). A file Koppel 0.1.0 wrote is carried over on the first start: the certificate, the signing key, and the clients whose secret hash Skakel reads (a client added before Koppel kept secrets in their current form is dropped, and logged). A 0.1.0 file that cannot be carried over is copied aside as `secrets.json.0.1.0-<UTC time>` and the server starts with a new certificate, a new key and no clients, which the log says.
- `connection.json` (client role): server address, pinned fingerprint, client ID and secret, and during a certificate handover the server's next fingerprint. Encrypted. Its format did not change in 0.1.1, so a client keeps its connection, and once its server runs 0.1.1 too it links as before.
- `ui-auth.json`: the UI password hash and security stamp. Encrypted.
- `devices.json` (server role): the bus IDs the user chose to share, with their VID:PID. Plain.
- `attached.json` (client role): the devices this client attached, with the server they belong to. The client attaches them again whenever the server offers them and nobody is using them: after a restart, an upgrade, a dropped link, or a replug on the server. Detaching forgets the device. Plain.

How a file is read and written:

- **Start-up never replaces a file.** Only a missing file is seeded with defaults. A file that exists but cannot be used (unreadable after a few retries, cannot be decrypted, is not valid JSON, or fails validation) stops start-up with an error that names the file, the cause and the likely fix, and the file is left exactly as it was.
- **Two plain files start with defaults instead.** `devices.json` and `attached.json` are not critical and their defaults are safe (nothing shared, nothing remembered), so a corrupt one does not stop Koppel. It starts empty in memory, the file is left untouched, and `GET /api/system/checks` shows a `devices` or `attached` entry ("devices.json was not used.") with the reason. The first change made while the file is bad (sharing a device, attaching one) moves it aside as `<name>.unreadable-<UTC timestamp>`, logs a warning, and writes a fresh file; fixing the file by hand instead clears the entry. `settings.json` and the encrypted files still refuse to start: defaults would put a machine back in the Server role, or lose keys and credentials.
- **Hand edits are validated** the same way a REST change is, including the port rules. While Koppel runs, a bad edit is ignored: the last good value stays, a warning is logged, and `GET /api/system/checks` shows a `settings` entry, "settings.json was not applied", with the reason. A later valid edit clears it. A REST change made over a bad hand edit replaces the bad file with the last good value plus the change.
- **A valid hand edit** is loaded before the next change, so a change from the UI never overwrites it.
- **Pushes are ordered.** A change reaches subscribers in write order, one change at a time, even when several writes are in flight. A handler may update the same file: the nested change is written and applied at once, its delivery is queued behind the running one and anything already waiting, so every subscriber sees the same order, and the nested call returns without waiting for that delivery (it would wait for itself).
- **Writes** go to a random temp name, are flushed, and replace the file; a failed write removes its temp file, the replace is retried on a sharing error, and stale temp files are swept at start.
- **Enums are names only:** a number where a name is expected (`"role": 1`) is refused.

The encrypted files are sealed with a key in a separate keys directory (`Paths:Keys`): machine DPAPI on
Windows, and on Linux AES-256-GCM with a key file that is plain on disk (see "Secret files" in "Config
files in Rust"). 0.1.x used a Data Protection key ring there, which Koppel converts from once (see "The
upgrade from 0.1.x" there). `Paths:KeyCertificate`, which protected that key ring on Linux, is deprecated
and goes in 0.2.0, which cannot read a key ring protected with it.

Data folder hardening. Before anything is written, Koppel restricts the config and keys directories:

- **Windows:** the owner becomes Administrators and the ACL is protected (no inherited access) with full control for SYSTEM and Administrators only. A directory that is a junction or a symbolic link is refused. A process that is not elevated, such as a developer running from a console, cannot take ownership, so it also keeps its own access.
- **Linux:** `chmod 700`. A bind-mounted config directory that the process cannot chmod makes start-up fail.

Where the config directory is:

- Docker: `/koppel/config` and `/koppel/keys` inside the `koppel` volume.
- Otherwise: `%ProgramData%\Koppel\config` on Windows, `/var/lib/koppel/config` elsewhere, unless `Paths:Config` is set. A developer running `cargo run -p koppel` sets `Paths__Config` and `Paths__Keys` to a folder of their own.

### If the key ring is lost

The encrypted files can be read only with the key ring that wrote them. If the keys directory is
missing, replaced or restored from another machine, Koppel refuses to start and says which file it
cannot use ("Koppel cannot use ...secrets.json: ... The file was not changed ... restore the keys
directory"). It does not make new secrets over the old ones.

- **What you see:** in Docker the container exits and restarts, over and over (`restart: always`), with that message in `docker logs`, until the keys are back. On Windows the service fails to start and the recovery rule retries it; run `Koppel.exe` from an elevated prompt with the service stopped to read the message.
- **What to do:** restore the keys directory from a backup (the keys directory and the config directory belong together). If the keys are really gone, the encrypted files cannot be recovered: move the unreadable ones aside (`secrets.json`, `connection.json`, `ui-auth.json`) and start again. A new `secrets.json` means a new signing key and a new certificate, so every client must be paired again. A lost `connection.json` means entering or pairing the connection again. A lost `ui-auth.json` means setting the password again, within the setup window after the start.

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
  itself when the folders are in it) and `chmod 700` on Linux.
- **Session key:** `ui-session.key` in the keys directory, sealed the same way, signs the UI's session cookies (see ["The UI's server side in Rust"](ui.md#the-uis-server-side-in-rust)).
- **Secret files:** a six byte header, `KPSF`, a format version (1) and a scheme. Scheme 1, on Windows,
  is machine DPAPI over the JSON. Scheme 2, elsewhere, is AES-256-GCM with a random key in
  `koppel-secrets.key` in the keys directory, mode `0600`; the file then holds that key's id and a
  nonce before the ciphertext. Data Protection's key ring is no longer used. As before, anyone who can
  read the keys directory on Linux can decrypt the files.
- **The upgrade from 0.1.x:** at the first start, each of `secrets.json`, `connection.json` and
  `ui-auth.json` that is still a Data Protection payload is decrypted with the 0.1.x key ring (machine
  DPAPI on Windows, plain XML on Linux) and written again in the new format, with the same JSON inside.
  All are decrypted before any is written, so a failure changes nothing: the log says why, and the
  start then stops on the first file still in the old format. The key ring is left as it was. A key
  ring protected with `Paths:KeyCertificate` is not read: its files are renamed to
  `<name>.0.1-key-certificate-<UTC time>`, unchanged, and Koppel starts as a new install, so every
  client pairs again and the password is set again.
- **What carries over:** the certificate, so pinned clients keep their pin; the clients and their
  secrets; the client's connection; the UI password. The token signing key stays in `secrets.json`
  but does not need to carry over: tokens last 30 minutes and live only in a client's memory, a 0.2
  client cannot use a 0.1.x server's tokens anyway (Skakel protocol 2), and Skakel's Rust server signs
  with ECDSA P-256 where 0.1.x used RSA, so 0.2 starts with a new key. UI sessions end once: 0.1.x's
  cookies were protected by keys in `keys/aspnet`, which 0.2 does not read.
- **Certificate:** 0.1.x kept it as a password-less PKCS#12 (3DES and SHA-1, where "no
  password" is an empty BMPString without its terminator). The PKCS#12 crates refuse that form, so
  `koppel-config` reads it itself and hands the DER certificate and PKCS#8 key to rustls.
