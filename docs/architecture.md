# Koppel architecture

Koppel shares USB devices over the network. A machine runs as a **server** (offers the devices
plugged into it), a **client** (attaches devices from a server so they appear local), or both.
One codebase and one Docker image cover every role; the role is a setting.

## Layers

| Layer | What | Status |
|---|---|---|
| Control plane | REST for commands, a live push for confirmed state, React UI behind a password: axum with Server-Sent Events | Done, with the roles on Skakel 0.2.0 (see ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust)) |
| Transport | [Skakel](https://github.com/matumi-software/skakel) (MIT): discovery, pairing, OAuth client credentials, certificate pinning and handover, the remote hub and the streams. Koppel keeps the server's state and the client's credentials in its own encrypted files. The UI has its own password | Done |
| Data plane | USB/IP protocol (Linux kernel spec), in Koppel's `usb` stream on Skakel | Linux server and client work. A Windows client works through usbip-win2 (see "Real hardware" in [usb.md](usb.md#tested)). A Windows server shares through usbipd-win, and a real one was run to a Linux client |

## The documents

| Document | What it covers |
|---|---|
| [skakel.md](skakel.md) | What Skakel does and what Koppel does, finding servers and pairing, and the roles on Skakel in Rust |
| [ui.md](ui.md) | The confirm rule, the activity log, and the UI's server side in Rust |
| [ports.md](ports.md) | The ports and what each one serves |
| [security.md](security.md) | Auth, sign-in, the guards on the web surface, certificate rotation and the threat model |
| [usb.md](usb.md) | The USB data plane, and USB in Rust |
| [config.md](config.md) | Config on disk, and the config files in Rust |
| [installers.md](installers.md) | Running Koppel on another machine, and the installer and Windows service |
| [building.md](building.md) | Building and releasing, the Rust workspace, release sizes, Linux packages and the image, and the contract tests |
| [decisions.md](decisions.md) | Known limits and the USB feasibility study |
| [windows.md](windows.md) | Running Koppel on Windows through Docker |
