# Changelog

## 0.1.0 (2026-11-02)

First release. If you ran a development build, read Installer and deploy before upgrading.

### Security

- The UI now needs a password. The first password needs a one-time setup code (`docker logs`, or `%ProgramData%\Koppel\config\setup-code.txt` on Windows). Wrong tries are slowed down, and sessions end when the password changes or is reset (`Koppel --reset-password`).
- The UI and the remote API are on separate ports. The UI (5240, HTTP) never serves the token endpoint or pairing, and the HTTPS port (5241) never serves the UI. Other websites cannot drive the UI (Origin check, required `X-Koppel-UI` header, SameSite cookie), and responses carry security headers. The Windows installer binds the UI to this machine only.
- Revoking a client, or rotating its secret, now ends its running USB sessions and hub connections at once, not only new requests.
- Pairing is harder to abuse: a secret poll token, a server-wide cap on requests, per-address limits, and a confirm step before a client replaces its connection to another server. Discovery results are size-limited and expire.
- Client secrets are verified with SHA-256 (older hashes are upgraded on use). The server still keeps them recoverable in its encrypted file, so an admin can copy a secret again.
- The token and pairing endpoints are rate limited. The config and keys folders are locked down to administrators on Windows and `chmod 700` on Linux.

### Reliability

- Koppel no longer replaces an unreadable or invalid config file with defaults. It stops with a message naming the file and leaves it alone. If the key ring is lost, the container restart-loops until the keys are restored (see `docs/architecture.md`).
- A bad hand edit of `settings.json` while Koppel runs is ignored, with a system check that says why. Edits are validated like UI changes.
- Config changes reach the UI in the order they were written.
- Writes use unique temp files and survive a briefly locked file.

### USB

- An attach that stalls now fails after 20 seconds (`Usb:AttachTimeoutSeconds`) with a readable reason, instead of hanging.
- A saved-connection or role change detaches everything first. Remembered devices are kept and attach again.
- A server only exports a device that is bound and is the device you shared on that port. At start it unshares anything it does not mean to share. Leaving the server role unshares and closes the USB port.
- The USB port cannot be set to Koppel's own web or gateway ports.
- The gateway runs only in the Client or Both role and retries a taken port. On native Windows it serves only the kernel driver's connections.
- Stopping Koppel detaches devices attached through the Windows driver. Bus IDs are validated everywhere. Connections use TCP keepalive.
- The USB port's state shows as a `listen` system check.
- Tested with fakes, not yet re-run on real hardware. The earlier real-hardware result (a Logitech receiver working as keyboard and mouse on a Windows 11 client) still stands.

### UI

- Sign-in and sign-out, a password change panel, and a first-run password step.
- Hub pushes apply everywhere and pending changes survive leaving a page, so a change is never shown as saved before the server confirms it.
- Pairing shows the server's fingerprint, warns about duplicate names, and has Replace and Discard for a replaced connection. Denied requests show why.
- Copy works on plain HTTP. The connection test reports one of a fixed set of outcomes.

### Installer and deploy

- Install fails, with instructions, if `%ProgramData%\Koppel` is a link or is owned by someone who is not an administrator. A folder owned by an administrator is taken over. A machine where an ordinary user owns the folder needs it renamed once.
- The driver install writes to the MSI log, not to a file. Setup may ask for a restart. A usbip-win2 you installed yourself is used and left alone, and uninstall removes only what Koppel installed.
- The service restarts after a crash. Firewall rules are split into domain and private.
- Docker deploy composes pin `matumisoftware/koppel:0.1.0`. To upgrade, edit the tag, then `docker compose pull && docker compose up -d`. `deploy.sh` images are never replaced by a pull.
- Builds are pinned: base images, the vendored usbip-win2 files (hashes and signers checked), and package lock files. `THIRD-PARTY-NOTICES.txt` ships in the installer and the image.
- `prepare-host.sh` is run with `sudo bash`, installs Docker from Docker's apt repository and hints at `linux-modules-extra` when a module will not load.
- The image has a health check on `/healthz`.

### Known limits

- The Linux container is privileged and runs as root. The Windows service runs as LocalSystem.
- The installer is not signed, and there is no CI.
- The elevated installer paths (taking over a data folder, driver record, rollback, restart request) have not been run on a VM.
- The UI port is plain HTTP: bind it to loopback or use a TLS proxy on a network you do not trust.
- Windows can be a client; a Windows server needs usbipd-win and `Usb:Upstream`, and has not been tried with real usbipd-win.
- Storage and printers (bulk transfers) are untried on real hardware.
