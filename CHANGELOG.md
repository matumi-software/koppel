# Changelog

## 0.1.2

- The Windows app has a Koppel icon in the notification area for every signed-in user, the way Apollo does it. Click it
  to open the UI in your browser. Its menu also shuts Koppel down, after asking first; Koppel then stays off until the
  next boot or until the service is started again. The icon comes and goes with the service.
- The tray icon's tooltip says how Koppel is doing: what a client is linked to and how many devices it attached, or
  what a server shares and how many are in use. An amber dot on the icon means the link is down, sharing is not
  available, or setup is waiting.
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
