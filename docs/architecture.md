# Koppel architecture

Koppel shares USB devices over the network. A machine runs as a **server** (offers the devices
plugged into it), a **client** (attaches devices from a server so they appear local), or both.
One codebase and one Docker image cover every role; the role is a setting.

## Layers

| Layer | What | Status |
|---|---|---|
| Control plane | ASP.NET Core host, REST for commands, SignalR for pushes, React UI | Scaffolded |
| Auth | The server is an OAuth server (client credentials); client ID and secret are kept encrypted next to the config on both sides | Planned |
| Data plane | USB/IP protocol (Linux kernel spec), wrapped in TLS | Planned |

## The confirm rule

The UI never assumes a change worked.

1. The UI sends the change over REST (`PATCH /api/settings`). The server answers `202 Accepted` with no body.
2. The server writes the change to its JSON config file (temp file, flush, move over the target).
3. Only after the write lands does the server push the confirmed state over SignalR (`SettingsChanged` on `/api/hubs/ui`).
4. The UI shows the control as pending from step 1 until the pushed value matches what the user asked for. A rejected request (400) or no confirmation within 10 seconds puts the control back to its confirmed value with the reason.

Hand edits to the config file are picked up by a file watcher and pushed the same way. While the
SignalR link is down, every control that sends a change is disabled, and the UI refetches over
REST after reconnecting.

Code: `src/Koppel/Settings/SettingsStore.cs` (write, then raise), `src/Koppel/Hubs/UiHub.cs`
(relay to the hub), `src/Koppel.UI/src/settings/use-settings.ts` (pending, confirm, fail).
The end-to-end test is `tests/Koppel.Tests/Settings/SettingsConfirmFlowTests.cs`.

## Config on disk

`settings.json` lives in the config directory:

- Development: `.koppel/config` at the repo root (gitignored), from `appsettings.Development.json`.
- Docker: `/koppel/config` inside the `koppel` volume (`Paths__Config`).
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
