# USB

## USB data plane

### USB server

- **Host:** `UsbHost` says what a host can do instead of failing or pretending: whether it can share, unshare and reset, and the sharing advice (the command to run on the machine that does the sharing, such as `usbipd bind`). An export readies the device first, which is when Koppel answers the client's import request, and then relays until the session ends (see "Export in two steps" in ["USB in Rust"](#usb-in-rust)).
- **Linux host:** `UsbServer` polls sysfs every two seconds and applies the share choices kept in `devices.json`, which holds the bus IDs and the VID:PID each had when shared. It re-shares a device when it is plugged back in. `SysfsUsbHost` binds a device to `usbip-host`, and takes the bus ID off `match_busid` again when a bind fails. The poll survives any error from the host: it logs it and asks less often until the host answers again.
- **Pushing the list:** `UsbServer` is one actor task that owns the share state and takes every change as a command (a share choice, the poll, a role change, a client's import or release), so two of them never change the host at once. It publishes each snapshot of the device list through a `tokio::sync::watch` channel, and each subscriber gets the newest one when it falls behind. A handler that never returns holds up only its own subscriber, never an import, a share or the poll. An import or release sends a command and does not wait for it. One scan of the host per poll tick feeds both the sharing and the snapshot; it looks again only when the sharing changed something. A device whose state cannot be read shows a fault; the others still list.
- **Exportable rule:** a device can be exported only when it is bound and is the VID:PID that was shared on that bus port. A bus ID is a port, so a different device plugged into the same port is unbound, shows a fault and is not shared without asking. A share whose apply fails is not saved. Bus IDs are parsed into a `BusId` before anything is saved; the endpoint answers 400.
- **Start-up reconciliation:** at start, Koppel shares what was saved (when `ShareOnStartup` is on and the role serves) and unbinds every other device bound to `usbip-host`, such as a share a crash left behind. A sysfs error here is logged and does not stop start-up; the poll applies the saved shares on its next pass.
- **Role change:** the surface guard serves the data plane only while the role is server or both. Leaving the server role ends the USB streams and every client's calls to the server (see "Role change" in ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust)), and unshares the devices in the background, so the settings change is confirmed to the UI without waiting for the unshare.
- **Session:** Koppel's stream type `usb` on Skakel's streams, on the network port, pinned to the server's certificate and with the client's bearer token. Skakel checks the token and that the client is registered at its current generation before the stream opens, and `UsbDataPlane` then speaks plain USB/IP inside the stream's data frames: a device list, or an import followed by the device's traffic. The type has no target and no control messages of its own. The client has 10 seconds to ask for one. A session ends when its client is revoked or rotated, when the role leaves the server side, when the connection under it dies, or when Koppel stops (see "USB streams" in ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust)).
- **Import:** the status check, the reset and the start of the export run under a lock per bus ID, so two imports of one device cannot race the reset. An upstream host (usbipd-win) claims the bus ID before it connects to the upstream server and releases it after the connection is closed.
- **Relay:** on import the server writes one end of a loopback socket pair to the device's `usbip_sockfd`, so the kernel serves USB from there, and relays the other end through the stream. The loopback pair accepts only the kernel end's own connection. The connection pings after 15 seconds of quiet and is dead 15 seconds later, so a client that vanished frees its device.
- **Revocation:** see ["Auth"](security.md#auth). Skakel ends a client's USB streams the moment it is revoked or rotated.
- **Live list:** clients follow the server's shared devices through Koppel's own gRPC service, `RemoteDevices` (HTTPS, bearer token, pinned certificate; see "Remote device list" in ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust)). A new call gets the list at once. The remote view shows the shared devices with their bus ID, VID:PID, name, manufacturer, speed, state and interfaces. It blanks who holds a device, since when, the serial number, the driver and any fault text: another client learns only that the device is taken.
- **Docker:** USB needs a privileged container running as root, and the host must have the modules loaded (`modprobe usbip-host` on servers, `modprobe vhci-hcd` on clients). Attached devices appear on the client's host kernel.
- **Windows:** the container uses a USB/IP driver on the Windows host over plain USB/IP inside the machine. A server shares what usbipd-win exports (`KOPPEL_USB_UPSTREAM`, `UsbIpUpstreamHost`). The upstream host refreshes in the background with one deadline, so the UI reads a cached snapshot, and caps the number of devices and the bytes it accepts. `KOPPEL_USB_UPSTREAM` is `host`, `host:port`, `[v6]:port` or a bare IPv6 address. A native Windows server still needs `KOPPEL_USB_UPSTREAM` pointed at usbipd-win. See `docs/windows.md`.

### USB client

- **Attach:** the client opens a `usb` stream on its link, imports over it, writes its loopback end to `vhci_hcd`'s `attach` (Linux) or hands the session to usbip-win2's driver (Windows), and relays. One deadline covers the token, connecting, TLS, the stream, the import reply and the device record: 20 seconds, `KOPPEL_USB_ATTACH_TIMEOUT_SECONDS`. A stall becomes a readable error on the device instead of a hang. The import reply must match the bus ID and VID:PID of the device the person picked. Bus IDs from the server are parsed into a `BusId` on receipt (while reading the list, or the import reply) and a row with an invalid one is dropped.
- **One link:** a saved connection has one Skakel link (`SkakelLink`), with its token and pinned connection, shared by the device list, the attaches and the gateway. A new server or new credentials get a new link; the same server at an address the person typed gets a new link and keeps its attached devices. Each USB stream's connection pings after 15 seconds of quiet and is dead 15 seconds later.
- **Fail visibly:** every background loop (link, attach, gateway) catches everything, logs it, shows the state and backs off. Failures use Skakel's fixed wording, for example a certificate that no longer matches the pinned fingerprint, and a revoke or rotation on the server says to pair again.
- **Attachments:** each device is one slot in a state machine (`client/slots.rs`: attaching, attached, releasing, or faulted), keyed by the connection epoch plus the bus ID. The epoch changes when the saved connection moves to another server or credentials, so two servers with the same bus ID cannot be mixed up, and a server switching to a new certificate or moving to another address keeps its epoch and its attachments. One actor task (`client/actor.rs`) owns the slots, the link and the device list; the slow work (opening a stream, the driver's command, undoing an attach) runs in tasks that report back with a command, so an attach that is no longer wanted finds out from the machine, not from a check made afterwards. Asking to attach a device that is attached or on its way is a no-op.
- **Connection or role change:** when the saved connection changes to another server, or the role leaves client and both, Koppel detaches and stops everything first. The devices stay remembered in `attached.json`, so they attach again when the new link is up. The release runs in the background (the driver's own command can take 30 seconds), so the settings update, and the UI's confirmation, return at once; an attach that starts meanwhile waits for it. Detaching by hand releases the device first and forgets it after: if `attached.json` cannot be saved, the device is detached and shows a fault. A detach during an attach that is still in flight is noticed when the driver returns, and the driver's attachment is released.
- **Gateway:** `UsbIpGateway` runs only while the role is client or both. It is a plain USB/IP server for a USB/IP driver on this machine (see ["Windows"](#windows) below). It serves at most 64 sessions at once (`KOPPEL_GATEWAY_MAX_SESSIONS`) and closes the rest at once. If the port cannot be bound, it tries again with a wait that grows from 5 to 60 seconds, and the reason shows as `gatewayFault` in the client's device list (the devices page shows it, and `/readyz` reports the gateway as `faulted`).
- **Native Windows:** the gateway accepts only connections owned by the kernel (PID 4, which is how usbip-win2's driver connects), looked up in the TCP table. Another process that reaches the loopback port is turned away. This applies on native Windows with an IPv4 loopback address. An IPv6 loopback address is refused outright with a warning. Where a driver is present, the gateway also serves only imports Koppel asked for: before it runs `usbip attach` it books one import for that bus ID, and the gateway accepts exactly one import for the booking. Another local user's `usbip attach`, a device list request, and the driver's own attempts to attach again after a drop are refused; Koppel attaches a remembered device again itself. Elsewhere, such as the Linux container of Docker Desktop, the connection does not come from this machine's process table, so no check applies and the compose file's port publishing is what keeps it local.
- **Stopping:** stopping the client detaches every attachment made through the Windows driver (`usbip attach --stop`, then `detach -p`) and stops every relay, so a Linux shutdown does not wait on them. Devices stay remembered. A `--stop` that fails because there was nothing to stop does not fail the attach or the detach.
- **Windows driver:** `UsbipWin2Driver` runs usbip-win2's `usbip.exe`. The bus ID is checked before it is passed, as `--bus-id=<id>`.

### Tested

- **Running the tests:** `cargo test --workspace` runs everything. Skakel's own tests cover discovery, pairing, tokens, the handover and the streams; Koppel's cover what it builds on them.
- **Fakes:** the protocol encoding, the sysfs adapters against a fake `/sys`, and a full server-to-client run on real TLS with fake kernels on both ends (token, pinning, hub, stream, import, byte round trip). The server and client changes listed above (deadlines, revocation, the exportable rule, reconciliation, the gateway check, detach on stop) have fake-based tests, and most were also run on real hardware on 2026-11-03 (below).
- **Windows client through Docker Desktop (2026-11-03):** `usbip.exe attach -r 127.0.0.1` through the container's gateway attaches without elevation, Windows enumerates the device, Koppel shows it attached through the gateway, and Koppel's Detach removes it.
- **Not exercised on real hardware:** the attach deadline on its own (the gateway only served the kernel's connections).
- **Real kernel (2026-11-02, WSL2 6.6, software USB device):** sharing binds the device to `usbip-host`, both kernels take Koppel's sockets, and the device enumerates through both relays and TLS up to `SET_CONFIGURATION`. That step deadlocks because the test device and its attached copy sit on the same virtual controller, the only one the WSL kernel has; the same happens with no Koppel in the path. Configuration and bulk data still need a client on a second machine.
- **Real hardware (2026-11-02, first run):** a Debian thin client runs the server in Docker and shares a Logitech USB receiver. A Windows 11 client attaches it through the installed service and usbip-win2, and through Docker Desktop's gateway, and it works as keyboard and mouse there.
- **Real hardware (2026-11-03):** a Debian 13 machine runs the server in Docker and shares two Logitech receivers, a flash drive and a webcam. A native Windows 11 client attaches the receivers and the drive, and they keep working across an installer upgrade. A 256 MB file written to the drive and read back after a detach and reattach matched its checksum (about 8 MB/s). A 75 second network drop was recovered from by itself: the server freed the devices after about 50 seconds and they reattached within 2 seconds of the traffic returning. Rotating a client's secret ended its sessions at once, and stopping the client service removed the devices. A Windows 11 PC running a real usbipd-win shared a webcam to the Debian machine, which streamed it at 30 fps, so isochronous transfers cross the relay in that direction. A Logitech C270 webcam from the Linux server to the Windows client attaches but delivers no frames (#22).
- **Linux package (2026-11-03):** the amd64 `.deb` was installed, upgraded, removed and purged on a Debian 13 machine with real USB. The service started, the setup code (as first-run setup worked then) was in `setup-code.txt`, an upgrade kept the certificate and password, and purge deleted `/var/lib/koppel`.
- **Windows installer (2026-11-03):** run on a clean Windows 11 22H2 VM: fresh install, service recovery after a killed process, major upgrade, uninstall, taking over an administrator-owned data folder, and refusing one owned by a non-administrator. The rollback of a failed fresh install was not exercised.
- **Untested:** printers, and the arm64 image and `.deb` on a Raspberry Pi.

### Windows

The container uses a USB/IP driver on the Windows host over plain USB/IP inside the machine. A server
shares what usbipd-win exports; a client runs a gateway that usbip-win2 attaches from
(`KOPPEL_GATEWAY_PORT`, `UsbIpGateway`). See `docs/windows.md`. Installed natively, a Windows client
attaches through usbip-win2's driver, which the installer bundles; on native Windows the gateway is
on by default (port 3242, loopback). A native Windows server still needs `KOPPEL_USB_UPSTREAM` pointed at
usbipd-win.

### USB in Rust

`koppel-usb` is the USB layer, by the rules in ["USB data plane"](#usb-data-plane) above: the exportable rule, start-up reconciliation, the import lock per bus ID, the
attach deadline, `attached.json` and attaching again after a drop, the gateway and its checks. Those
sections describe the behaviour; this one says where it lives in Rust and what differs.

| Module | What |
|---|---|
| `usbip` (its own crate) | The USB/IP device list and import messages (`protocol`), byte for byte, `BusId`, a bus ID that can only be made from valid text, the device model (`UsbDevice`, the export and virtual port states) and `relay`, which copies both directions with one write and one flush per read. koppel-usb re-exports them. No Koppel crate and no `tokio-util` in it, and the properties of the messages are checked there with proptest |
| `host.rs` | `UsbHost`, `UsbAttacher` and `UsbIpDriver`: what a platform can do; `Capabilities`, the value that says what a host can do; and `Platform`, the bundle of the three that Koppel is started with (a part the machine lacks is `None`, and the platform gives one reason). The kernel attach and detach write to sysfs, so `Platform` runs them on `spawn_blocking` |
| `linux` (Linux only) | `SysfsUsbHost` (usbip-host) and `SysfsVhci` (vhci-hcd) through sysfs, and the usbfs reset |
| `windows` (Windows only) | `UsbipWin2Driver`, which runs `usbip.exe`, and `WindowsTcpTable` |
| `upstream` | `UsbIpUpstreamHost` for usbipd-win, and `UpstreamAddress` (`KOPPEL_USB_UPSTREAM`) |
| `stream`, `relay` | The stream to the other machine, an in-memory pair for tests, and the loopback socket pair |
| `server`, `data_plane` | `UsbServer` and `UsbDataPlane` |
| `client/` (`mod`, `actor`, `slots`, `attach`, `gateway_session`), `gateway`, `peer_check` | `UsbClient` (made from a `Deps` struct), its actor, the per-device state machine, the gateway's sessions, `UsbIpGateway`, and which connections the gateway serves |
| `store` | `Role`, `UsbSettings`, `Connection`, `SharedDevices` and `AttachedDevices`, the values the roles read and save, and the `Observed` and `Store` traits they read and save them through |
| `views`, `checks`, `activity` | What the UI sees, with 0.1.x's JSON names, the USB system checks, and the activity events |

- **The stream to the other machine:** the crate does not know Skakel. A `DeviceStream` is the byte
  stream for one device: tokio's `AsyncRead` and `AsyncWrite` to read and write, and `close(reason)` to
  end it, with `CloseReason::Done`, `Stopping` (the role left the server side) or `Failed`. On the server
  the transport accepts a client's stream after its own checks and hands it to `UsbDataPlane::serve`
  with the client's ID and name. On the client the link to the server is a `Link`, a `StreamOpener`
  that also starts, stops and says its address, made by the `Links` the transport provides; the link
  reports its state and each device list through `LinkEvents`. `koppel-web` implements these on
  Skakel's gRPC streams, one connection per device stream (see ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust)). The
  crate's own tests use the in-memory pair, and real config files behind the store traits (`tests/common/files.rs`, a test copy of the program's glue).
- **Export in two steps:** `UsbHost::export` readies the device (the kernel holds its socket, or
  usbipd-win accepted the import) and returns an `Export`; the data plane answers the import and then
  calls `Export::relay`.
- **Left to the transport:** keeping streams out while the role is not a server, revoking and rotating, pings, renewing the token, following a moved server and saving its
  address (`serverMoved`), and saving the certificates the server announces; see "The roles on Skakel
  in Rust". The client still carries its remembered devices over to the server's next certificate in
  `attached.json`.
- **Settings and files:** the crate names no config file. The server, the data plane, the client and
  the gateway follow the settings (`Observed<UsbSettings>`) and the saved connection
  (`Observed<Connection>`), and save the shares and the attached devices into a `Store<SharedDevices>`
  and a `Store<AttachedDevices>`. `koppel-app/src/roles/usb_store.rs` puts those on `settings.json`,
  `connection.json`, `devices.json` and `attached.json`, through `koppel-config`'s `ConfigFile`. A change
  is delivered through the file as before, so a settings update still returns once the roles have applied
  it. `KOPPEL_USB_ATTACH_TIMEOUT_SECONDS` and the `Gateway:` settings come in as
  `ClientOptions` and `GatewayOptions`. The activity log implements `ActivitySink`.
- **The gateway's check:** on Windows with a loopback address it looks the connection up in the TCP
  table. Koppel ships no Windows container, so it does not check whether it runs in one.
- **Tests:** with fakes: a kernel on both ends, a usbip-win2 driver, batch files
  in place of `usbip.exe` (Windows only), a stand-in USB/IP server for usbipd-win, a fake `/sys` tree
  (Linux only, as the code is, and the folders have `:` in their names), and a server and a client
  joined in memory. The cases that need Skakel (pinning, revoking, the certificate handover, a moved
  server) run in `koppel-web/tests/roles.rs`, with the fake kernels on both ends. On real hardware with 0.2.0, only a Debian 13 server in Docker
  sharing to a native Windows 11 client has run (2026-11-09); a Linux client and a Windows server through
  usbipd-win have not (#86, #87, tracked in #115). The hardware runs under "Tested" above are of the C#
  0.1.x build.
