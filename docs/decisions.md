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
- **No command id in the 202 answer, and no `ack` on the push.** The idea was that the answer to a change names the change and the pushed value carries it back, so the UI would not have to compare values. It would change the wire on both sides and in the contract tests, and the comparison it replaces is small, covered by tests and has one useful property: it also confirms a change made by hand in the config file or from another browser. Comparing values stays.
- **Fonts are Latin and Latin Extended, woff2 only.** Every font file is embedded in the program, and the full `@fontsource` imports pulled in every subset and a `.woff` copy of each. Instrument Sans only has those two subsets anyway. JetBrains Mono also has Cyrillic, Greek and Vietnamese; a name in one of those scripts falls back to the system font in the `--font-sans` and `--font-mono` stacks, which draws all three. The ten files are in `react-ui/src/styles/fonts.css` with the packages' own unicode ranges. If the owner wants those scripts in the mono face too, add the faces there: about 20 KB per weight and script.

## koppel-usb 0.2.1 decisions

- **States and ids are types.** `DeviceState`, `LinkState` and `EntityKind` serialise to the same words as before (`local`, `shared`, `attached`, `offline`; `unconfigured`, `connecting`, `connected`, `error`; `device`, `link`), so nothing on the wire or in the UI changes. A state word this version does not know reads as `local`. `BusId` can only be made from a valid bus ID, and reading a device list leaves out a row whose bus ID is not one instead of failing the whole list. `UsbError` is a `thiserror` enum whose `Display` is the sentence the UI shows and whose I/O source stays for the log.
- **Left for the web side.** The pairing and link string states in koppel-web (`PairingState`, `roles/skakel/client.rs`), the 404 match, `RoleError` and `StartError` are koppel-web's and were not touched. `LinkStatus::Down(String)` and `OpenError::{Refused, Failed}(String)` keep their strings: the transport (koppel-web) makes them from Skakel's fixed wording.
- **One actor per role.** `UsbClient` and `UsbServer` each have one task that owns the state and takes every change as a command. Each device on the client is a slot in a state machine with no I/O (`client/slots.rs`), keyed by the connection epoch, tested against a model with `proptest-state-machine`. The snapshot pump and `Handlers<T>` became `tokio::sync::watch`: a listener is one task per listener, not one per event, and sees the newest value.
- **The machine is named `SlotState`, not `DeviceState`,** because the UI's state word already has that name.
- **Dependencies.** `thiserror` (MIT OR Apache-2.0, already in the tree). `http` (MIT OR Apache-2.0, already in the tree through axum and hyper) reads a URL's authority and `Usb:Upstream`, in place of hand-written splitting. Dev only: `proptest` and `proptest-state-machine` (MIT OR Apache-2.0) with tokio's `test-util`. `cargo deny check` passes with the existing licence list.
- **`same_fingerprint` stays hand-written.** It compares two fingerprints on their hex digits only; no crate does exactly that, and the code is eight lines with a test.
- **`relay` is not swapped for `tokio::io::copy_bidirectional`.** The stream to the other machine frames its writes, and `relay` writes each read as one write and flushes it, one message per read, as 0.1.x did (pinned by `one_read_is_one_write_and_one_flush`). `copy_bidirectional` reads into 8 KiB and flushes only when the reader would block, which changes the frames. Whether that matters on a Skakel stream under real USB traffic needs a real device, so the swap waits for one.
- **`loopback_pair` is not swapped for a `socketpair`.** It would save a listener and a port per attach on Linux, but the pair is handed to the kernel (`vhci-hcd`, `usbip-host`), and only a real kernel proves that a socketpair works there.
- **Monotonic time.** Retry throttling, the upstream refresh and the poll back-off read `Clock::monotonic` (tokio's `Instant`), so a step of the wall clock cannot stall them. The wall clock stays for what people read.

## koppel-windows 0.2.1 decisions

- **The tray pipe is written from a thread per tray.** `TrayProcess::send` puts the line on a bounded channel (32 lines) and returns; a thread owns the write end of the pipe. A tray that does not read fills its queue and further lines are dropped, with a debug line, instead of blocking the launcher. A state is sent again on the next round, so nothing is lost that matters. Closing the pipe drops the sender, so the thread ends and the pipe closes after the queued lines.
- **The tray's lines are a tagged enum.** `wire::Line` has `State`, `Notice` and an `Unknown` variant (`#[serde(other)]`), so a newer service can add a type. A known type with a field of the wrong kind (a number for `tip`) is now skipped as a whole line, where a non-boolean `attention` used to read as false. `wire` builds on every platform, so its tests run in the Linux container.
- **Tray state lives behind `GWLP_USERDATA`.** The statics are gone. The window owns a `Tray` that `WM_NCDESTROY` frees. Windows does not say which notification was clicked, so the page of a click is the page of the newest notice, cleared when that notice times out or is hidden. A click on an older one from the Action Center opens the UI's start, not another notice's page.
- **`winreg` for the `UiUrl` value** (MIT, on windows-sys, already in the tree), in place of raw Registry calls. The `windows` Registry feature stays on because the bindings gate `ShellExecuteExW` behind it.
- **The health probe does not pin the certificate.** It sends nothing and only learns that something answers 200 on the UI's address; the browser that the Start menu entry opens afterwards checks the certificate itself. Pinning would need the fingerprint in a place every user can read and kept in step with renewal. The reasoning is in `health.rs`.
- **Unsafe is denied in `koppel-windows` except in the modules that call the Windows API** (`event_log`, `health`, `launcher`, `opener`, `shell`, `tray`). `service`, `ui_url` and `wire` have none.
- **The UI address passed to a tray is quoted**, and `ui_url::checked` refuses an address with a quote in it.
