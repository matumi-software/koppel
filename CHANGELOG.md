# Changelog

## 0.1.0 (2026-11-02)

First release. If you ran a development build, read Installer and deploy before upgrading.

### Security

- The UI now needs a password. The first password needs a one-time setup code (`docker logs`, or `%ProgramData%\Koppel\config\setup-code.txt` on Windows). Wrong tries are slowed down, and sessions end when the password changes or is reset (`Koppel --reset-password`).
- The UI is served over HTTPS with the server's own certificate, so the password and the session cookie no longer cross the network in clear and the cookie is Secure. The browser warns about the certificate the first time. Open `https://<machine>:5240`; `http://` is redirected there.
- The UI answers only the local network by default, judged by the caller's address as Sunshine does (`Ui:AllowFrom`: `local`, `lan` or `any`). The Windows installer sets `local`. Other websites cannot drive the UI (Origin check, required `X-Koppel-UI` header, SameSite cookie), and responses carry security headers.
- Revoking a client, or rotating its secret, now ends its running USB sessions and hub connections at once, not only new requests.
- Pairing is harder to abuse: a secret poll token, a server-wide cap on requests, per-address limits, and a confirm step before a client replaces its connection to another server. Discovery results are size-limited and expire.
- Client secrets are verified with SHA-256 (older hashes are upgraded on use). The server still keeps them recoverable in its encrypted file, so an admin can copy a secret again.
- The token and pairing endpoints are rate limited. The config and keys folders are locked down to administrators on Windows and `chmod 700` on Linux.

### Reliability

- Koppel no longer replaces an unreadable or invalid config file with defaults. It stops with a message naming the file and leaves it alone. If the key ring is lost, the container restart-loops until the keys are restored (see `docs/architecture.md`).
- A bad hand edit of `settings.json` while Koppel runs is ignored, with a system check that says why. Edits are validated like UI changes.
- Config changes reach the UI in the order they were written.
- Writes use unique temp files and survive a briefly locked file.

### Ports

- Koppel needs only 5240, TCP and UDP: the UI, tokens, pairing, the remote API, USB traffic and discovery share it. `Ports:Network` gives what other machines do a port of its own, `Discovery:Port` moves discovery, and `Ports:Address` binds them to one address. The old `Ports:Http`, `Ports:Https` and `Ports:HttpAddress` settings are gone. A client saved with a server at `:5241` needs its address changed, or that server needs `Ports:Network=5241`.
- USB traffic runs as a WebSocket on the network port (`/api/remote/usb`), with the same certificate pinning and token as the rest, so port 3240 and the `listenPort` setting are gone. An existing `settings.json` with `listenPort` still loads.
- Finding servers uses a small UDP probe and answer on the discovery port instead of multicast DNS on 5353. An answer is never larger than its probe, so a server cannot be used to amplify traffic.

### USB

- An attach that stalls now fails after 20 seconds (`Usb:AttachTimeoutSeconds`) with a readable reason, instead of hanging.
- A saved-connection or role change detaches everything first. Remembered devices are kept and attach again.
- A server only exports a device that is bound and is the device you shared on that port. At start it unshares anything it does not mean to share. Leaving the server role unshares and ends the USB sessions.
- The gateway runs only in the Client or Both role and retries a taken port. On native Windows it serves only the kernel driver's connections.
- Stopping Koppel detaches devices attached through the Windows driver. Bus IDs are validated everywhere. USB sessions ping both ways, so a vanished peer frees the device in under a minute.
- Tested with fakes, not yet re-run on real hardware. The earlier real-hardware result (a Logitech receiver working as keyboard and mouse on a Windows 11 client) still stands.

### UI

- Sign-in and sign-out, a password change panel, and a first-run password step.
- Hub pushes apply everywhere and pending changes survive leaving a page, so a change is never shown as saved before the server confirms it.
- Pairing shows the server's fingerprint, warns about duplicate names, and has Replace and Discard for a replaced connection. Denied requests show why.
- Copy works on plain HTTP. The connection test reports one of a fixed set of outcomes.

### Installer and deploy

- Install fails, with instructions, if `%ProgramData%\Koppel` is a link or is owned by someone who is not an administrator. A folder owned by an administrator is taken over. A machine where an ordinary user owns the folder needs it renamed once.
- The driver install writes to the MSI log, not to a file. Setup may ask for a restart. A usbip-win2 you installed yourself is used and left alone, and uninstall removes only what Koppel installed, and only packages Windows still lists as usbip-win2's. A restart needed by the uninstall is requested too. A file in the data folder that no administrator owns is renamed aside.
- The service restarts after a crash, and after it stops with an error. The installer adds inbound firewall rules for the Koppel program (TCP and UDP, Private and Domain networks only), removed on uninstall. The setup window's licence link opens `THIRD-PARTY-NOTICES.txt`.
- Docker deploy composes pin `matumisoftware/koppel:0.1.0`. To upgrade, edit the tag, then `docker compose pull && docker compose up -d`. `deploy.sh` images are never replaced by a pull.
- Builds are pinned: base images, the vendored usbip-win2 files (hashes, signer certificates and the unsigned DLLs' hashes checked), and package lock files. `THIRD-PARTY-NOTICES.txt` ships in the installer and the image.
- `prepare-host.sh` is run with `sudo bash`, installs Docker from Docker's apt repository and hints at `linux-modules-extra` when a module will not load.
- The image has a health check on `/healthz`, at the configured UI address. Plain Docker does not restart an unhealthy container.
- The image is built for amd64 and arm64 (64-bit Raspberry Pi OS). The build cross-compiles on the build machine, so an x64 machine builds both without emulation.
- Linux without Docker: a `.deb` for amd64 and arm64 (`installer/linux/build.ps1`) installs Koppel self-contained as the `koppel` systemd service, loads the USB/IP modules at boot, and keeps `/var/lib/koppel` until a purge. `sudo koppel --reset-password` resets the UI password.

### Known limits

- The Linux container is privileged and runs as root. The Windows service runs as LocalSystem.
- The installer is not signed, and there is no CI.
- The elevated installer paths (taking over a data folder, driver record, rollback, restart request) have not been run on a VM.
- The UI's certificate is self-signed. Accepting the browser's warning on a network you do not trust could let an impostor collect the password; the fingerprint under Clients is the one to compare.
- Windows can be a client; a Windows server needs usbipd-win and `Usb:Upstream`, and has not been tried with real usbipd-win.
- Storage and printers (bulk transfers) are untried on real hardware.
- The arm64 image and the arm64 `.deb` are untried on a Raspberry Pi. The amd64 `.deb` was installed, upgraded, removed and purged under systemd in a Debian 12 container, and installed and started on Debian 13 and Ubuntu 22.04 and 24.04, but not on a machine with real USB.
- There is no apt repository yet, so the `.deb` is installed from a file and upgraded the same way.
