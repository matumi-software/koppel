# Decisions and known limits

## Known limits

- The Linux container is privileged and runs as root. Narrowing it needs a test on a real kernel. The Windows composes also run as root, because the volume belongs to it.
- The installer, the image and the release files are not signed, and a release is built by hand from a checklist (see "To release" in [building.md](building.md)). The checks in building.md are run by hand before a pull request.
- The rollback of a failed fresh install was not exercised (see ["Tested"](usb.md#tested)).
- Printers are untried, and the arm64 image and `.deb` are untested on a Raspberry Pi.
- Webcams do not stream from a Linux server to a Windows client (#22); a Windows server through usbipd-win to a Linux client does.
- The attach deadline on its own was not exercised on real hardware; it is covered by tests with fakes.
- The UI does not show `gatewayFault` or the live `listen` status yet.

## USB layer decisions (from the feasibility study, 2026-11-02)

- **Protocol: USB/IP**, as documented in the Linux kernel. Standard clients (`usbip attach`, usbip-win2, Docker Desktop) work against it, and we do not write our own virtual USB driver. Koppel's value sits around it: auth, discovery, UI.
- **USB traffic does not go over the push or REST.** It uses its own TCP connection; on Linux the kernel takes over the socket. The push and REST stay the control channel.
- **USB/IP has no auth or encryption.** Koppel carries it in a Skakel stream (gRPC over TLS) with a small relay on each side and only allows an import after an OAuth check. The raw USB/IP port is never exposed to the network.
- **Platforms:**
  - Linux server: the kernel's `usbip-host`, driven through sysfs. Docker image, privileged.
  - Linux client: the kernel's `vhci-hcd`, driven through sysfs. Attached devices appear on the host kernel, not only in the container.
  - Windows server: start by driving an installed usbipd-win through its CLI (not bundled). Our own libusb/UsbDk engine only if that CLI becomes a limit.
  - Windows client: the usbip-win2 driver (BSD-2-Clause) in a native install.
  - macOS client: not possible (no supported virtual host controller).
- **Licence: GPL-3.0-or-later** (0.1.0 and earlier: MIT). Code from compatible projects, such as usbipd-win (GPL-3.0), may come in with its notices; GPL-2.0-only code, such as the Linux kernel's, may not. See the rule in `CLAUDE.md`.
- **Expectations:** dongles, serial adapters, HID, storage and printers work well. Webcams and audio (isochronous) work on a good LAN but not perfectly.

## UI decisions (2026-11-12)

- **No `id:` and replay from `Last-Event-ID` on the push.** A reconnect refetches every query, a handful of small GETs on a LAN. Replay would need the server to keep a buffer of events and say when the buffer no longer reaches back, and the UI would still need the refetch for that case. Revisit only if reconnect traffic shows up as a problem.
