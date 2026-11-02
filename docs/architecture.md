# Koppel architecture

Koppel shares USB devices over the network. A machine runs as a **server** (offers the devices
plugged into it), a **client** (attaches devices from a server so they appear local), or both.
One codebase and one Docker image cover every role; the role is a setting.

## Layers

| Layer | What | Status |
|---|---|---|
| Control plane | ASP.NET Core host, REST for commands, SignalR for pushes, React UI | Scaffolded |
| Auth | The server is an OAuth server (client credentials); client ID and secret are kept encrypted next to the config on both sides | Done |
| Data plane | USB/IP protocol (Linux kernel spec), wrapped in TLS | Linux done, untested on real hardware; Windows not started |

## The confirm rule

The UI never assumes a change worked.

1. The UI sends the change over REST (`PATCH /api/settings`). The server answers `202 Accepted` with no body.
2. The server writes the change to its JSON config file (temp file, flush, move over the target).
3. Only after the write lands does the server push the confirmed state over SignalR (`SettingsChanged` on `/api/hubs/ui`).
4. The UI shows the control as pending from step 1 until the pushed value matches what the user asked for. A rejected request (400) or no confirmation within 10 seconds puts the control back to its confirmed value with the reason.

Hand edits to the config file are picked up by a file watcher and pushed the same way. While the
SignalR link is down, every control that sends a change is disabled, and the UI refetches over
REST after reconnecting.

Code (under `src/Koppel/`): `src/Koppel/Settings/SettingsStore.cs` (write, then raise),
`src/Koppel/Hubs/UiHub.cs` (relay to the hub), `src/Koppel.UI/src/settings/use-settings.ts`
(pending, confirm, fail). The end-to-end test is `tests/Koppel.Tests/Settings/SettingsConfirmFlowTests.cs`.

## Auth

- **Ports:** HTTP 5240 serves the UI. HTTPS 5241 serves the token endpoint (`/connect/token`) and the remote API (`/api/remote/*`) that other Koppel machines call.
- **Certificate:** each server creates a self-signed certificate on first start. Its SHA-256 fingerprint shows in the server's Clients section. Clients pin it: they talk only to a server presenting that exact certificate.
- **Clients:** the server admin adds a client by name and gets a client ID and secret once. The server keeps only a PBKDF2 hash of the secret. Revoking a client blocks it at once, even if it still holds an unexpired token.
- **Tokens:** OAuth 2.0 client credentials, issued by OpenIddict in degraded mode (no database; clients come from our encrypted file). Access tokens are signed with an RSA key kept in the encrypted secrets file, last 30 minutes, and are only accepted over HTTPS.
- **Not covered yet:** the UI on port 5240 has no login. Anyone who can reach that port can manage the machine. Keep it on a trusted network or bind it to localhost until UI login lands.

## Finding servers and pairing

- **Discovery:** a server announces itself over multicast DNS as `_koppel._tcp` on its HTTPS port, with TXT records `v=1`, `name`, `fp` (certificate fingerprint) and `version`. A client lists what it hears under Configuration, Servers on your network. Announcing uses Makaretu.Dns.Multicast.New, browsing uses Tmds.MDns (both MIT): no single .NET package was popular, maintained and able to do both. `Discovery:Enabled=false` turns both off. Multicast DNS stays on the local network, so servers elsewhere are still entered by address.
- **Pairing:** instead of copying a client ID and secret, the client shows a six-digit code and a person types it on the server's Clients page, under Waiting to pair, as with Moonlight and Sunshine. On a match the server registers the client and hands it its credentials over HTTPS pinned to the announced fingerprint; they go straight into `connection.json`, and `ConnectionChanged` confirms it as for any saved connection.
- **Why the code can be trusted:** it is numeric comparison with a commitment, as in Bluetooth pairing. The client sends a hash of a random nonce, the server answers with its own nonce, then the client reveals its nonce. Both ends derive the code from the pinned certificate and both nonces (`PairingCode`). A man in the middle has its own certificate and must fix each nonce before it sees the other side's, so it cannot steer the two codes to match. Three wrong codes refuse the request; requests expire after five minutes; a client may withdraw its request.
- **Endpoints:** `/api/pair` (HTTPS only, no token, refused on a client-only machine) for the exchange; `/api/server/pairing` for the server's UI; `/api/client/discovery` and `/api/client/pairing` for the client's UI. Pushes: `ServersDiscovered`, `PairingStatusChanged`, `PairingRequestsChanged`.
- **Docker:** multicast DNS does not cross Docker's bridge network, so the Linux server compose file uses the host network. On Windows, Docker Desktop's host networking does not reach the LAN: use the native install there. The Windows installer adds a firewall rule for Koppel.exe, UDP 5353, local subnet only.

## USB data plane

- **Server:** `UsbServer` polls sysfs every two seconds, applies the share choices kept in `devices.json` (binds the device to `usbip-host`), and re-shares a device when it is plugged back in. `UsbIpTlsServer` listens with TLS on the USB port (setting `listenPort`, default 3240).
- **Handshake:** inside TLS the client sends `KOPPEL/1 <access token>`; the server checks the token and that the client is still registered, answers `OK` or `DENIED <reason>`, then speaks plain USB/IP (device list or import).
- **Relay:** on import the server writes one end of a loopback socket pair to the device's `usbip_sockfd`, so the kernel serves USB from there, and relays the other end through TLS. The client does the mirror image: it imports over TLS, writes its loopback end to `vhci_hcd`'s `attach`, and relays.
- **Live list:** clients follow the server's shared devices over the `/api/hubs/remote` SignalR hub (HTTPS, bearer token, pinned certificate).
- **Docker:** USB needs a privileged container running as root, and the host must have the modules loaded (`modprobe usbip-host` on servers, `modprobe vhci-hcd` on clients). Attached devices appear on the client's host kernel.
- **Tested:** the protocol encoding, the sysfs adapters against a fake `/sys`, and a full server-to-client run on real Kestrel with fake kernels on both ends (token, pinning, hub, TLS, import, byte round trip).
- **Real kernel (2026-11-02, WSL2 6.6, software USB device):** sharing binds the device to `usbip-host`, both kernels take Koppel's sockets, and the device enumerates through both relays and TLS up to `SET_CONFIGURATION`. That step deadlocks because the test device and its attached copy sit on the same virtual controller, the only one the WSL kernel has; the same happens with no Koppel in the path. Configuration and bulk data still need a client on a second machine. The rig is in `D:\testing\koppel-usb`.
- **Real hardware (2026-11-02):** a Debian thin client runs the server in Docker and shares a Logitech USB receiver. A Windows 11 client attaches it through the installed service and usbip-win2, and through Docker Desktop's gateway, and it works as keyboard and mouse there. That covers configuration and interrupt transfers on a second machine; bulk transfers (storage, printers) are still untried.
- **Windows:** the container uses a USB/IP driver on the Windows host over plain USB/IP inside the machine. A server shares what usbipd-win exports (`Usb:Upstream`, `UsbIpUpstreamHost`); a client runs a gateway that usbip-win2 attaches from (`Gateway:Port`, `UsbIpGateway`). See `docs/windows.md`. Installed natively, a Windows client attaches through usbip-win2's driver, which the installer bundles (`UsbipWin2Driver` runs its `usbip.exe`). A native Windows server still needs `Usb:Upstream` pointed at usbipd-win.

## Activity

The server keeps an activity log in memory (Koppel.Core.Activity, shaped like Timebase.Core.Activity):
sharing, plugging and unplugging, attaches and releases, client changes and settings changes, each with
an outcome and, when it failed, the reason. The UI loads it from `GET /api/activity` and follows
`ActivityRecorded` over the hub, so it is the same after a reload and on every device. It starts
empty when the process restarts. A change the UI sent shows as Waiting until the server logs it.

## Running on another machine

`src/Koppel/deploy` runs Koppel on a Debian or Ubuntu machine:

1. Copy `prepare-host.sh` to the machine and run it once with sudo. It installs Docker, loads
   `usbip-host` and `vhci-hcd` and keeps them loaded at boot.
2. Copy `deploy/docker-compose.yml` there and run `docker compose up -d`. It pulls
   `matumisoftware/koppel` from Docker Hub. To upgrade, `docker compose pull && docker compose up -d`;
   config and keys stay in the `koppel` volume.

To run your own build instead, with SSH access to the machine: `bash src/Koppel/deploy/deploy.sh
user@host`. It builds the image here, copies it over SSH, and starts it with
`deploy/docker-compose.yml`.

The image is built for this machine's architecture; the script stops if the target's differs.

## Config on disk

The config directory holds:

- `settings.json`: plain, may be edited by hand.
- `secrets.json` (server role): signing key, HTTPS certificate, registered clients. Encrypted.
- `connection.json` (client role): server address, pinned fingerprint, client ID and secret. Encrypted.
- `devices.json` (server role): the bus IDs the user chose to share. Plain.
- `attached.json` (client role): the devices this client attached, with the server they belong to. The client attaches them again whenever the server offers them and nobody is using them: after a restart, an upgrade, a dropped link, or a replug on the server. Detaching forgets the device. Plain.

The encrypted files use ASP.NET Core Data Protection with a key ring in a separate keys directory
(`Paths:Keys`, chmod 700 or a restricted ACL). On Windows the key ring is protected with machine
DPAPI. On Linux it is plain on disk unless `Paths:KeyCertificate` points at a certificate kept
outside the data volume (for example a Docker secret); Koppel logs a warning when it is not.

Where the config directory is:

- Development: `src/Koppel/.koppel/config` (gitignored), from `appsettings.Development.json`.
- Docker: `/koppel/config` and `/koppel/keys` inside the `koppel` volume.
- Otherwise: `%ProgramData%\Koppel\config` on Windows, `/var/lib/koppel/config` elsewhere, unless `Paths:Config` is set.

## USB layer decisions (from the feasibility study, 2026-11-02)

- **Protocol: USB/IP**, as documented in the Linux kernel. Standard clients (`usbip attach`, usbip-win2, Docker Desktop) work against it, and we do not write our own virtual USB driver. Koppel's value sits around it: auth, discovery, UI.
- **USB traffic does not go over SignalR or REST.** It uses its own TCP connection; on Linux the kernel takes over the socket. SignalR and REST stay the control channel.
- **USB/IP has no auth or encryption.** Koppel wraps it in TLS with a small relay on each side and only allows an import after an OAuth check. The raw USB/IP port is never exposed to the network.
- **Platforms:**
  - Linux server: the kernel's `usbip-host`, driven through sysfs. Docker image, privileged.
  - Linux client: the kernel's `vhci-hcd`, driven through sysfs. Attached devices appear on the host kernel, not only in the container.
  - Windows server: start by driving an installed usbipd-win through its CLI (not bundled, so its GPL licence does not reach us). Our own libusb/UsbDk engine only if that CLI becomes a limit.
  - Windows client: the usbip-win2 driver (BSD-2-Clause) in a native install.
  - macOS client: not possible (no supported virtual host controller).
- **Licence: MIT.** Never copy code from GPL projects such as usbipd-win; reading them for reference is fine.
- **Expectations:** dongles, serial adapters, HID, storage and printers work well. Webcams and audio (isochronous) work on a good LAN but not perfectly.
