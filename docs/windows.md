# Koppel on Windows

Koppel runs in a Linux container on Windows too, through Docker Desktop. The container cannot reach
the Windows USB stack by itself: Docker Desktop runs it in a Linux VM, so a device the container
attaches lands in that VM, not in Windows. Each side therefore uses a USB/IP driver installed on the
Windows host, and talks plain USB/IP to it inside the machine. Between machines it is still Koppel's
TLS with a token.

```
Windows server                                       Windows client
usbipd-win  <--plain-->  Koppel  <==TLS + token==>  Koppel  <--plain-->  usbip-win2
(host)     127.0.0.1    (container)    network     (container)  127.0.0.1   (host)
```

A Linux machine needs none of this. It runs the normal image (`src/installer/docker`), and the mixes
work: a Windows client with a Linux server, and the other way round.

## Build the image

From `src` in the repo:

```powershell
docker build -f installer\docker\Dockerfile -t matumisoftware/koppel:0.2.1 .
```

That builds the image under the tag both Windows compose files name, so a build made here is used as it is. To use the published image instead, skip this step and Docker pulls it.

## Windows server

1. Install usbipd-win, in an administrator PowerShell:

   ```powershell
   winget install usbipd
   ```

2. Find the device and share it. Sharing survives a reboot.

   ```powershell
   usbipd list
   usbipd bind --busid 2-3
   ```

3. Start Koppel:

   ```powershell
   docker compose -f src\installer\docker\docker-compose.windows-server.yml up -d
   ```

4. Open https://localhost:5240/, accept the certificate warning and finish setup as a server. The devices page shows what
   usbipd-win shares. Share and unshare with `usbipd bind` and `usbipd unbind`; the switch in
   Koppel cannot change it. USB/IP sends no product names, so devices show by VID:PID.

The client compose file publishes 5240 as `127.0.0.1:5240:5240`, so the UI answers on this machine
only. The server compose file publishes 5240 on every address, because clients reach the server
there, and the UI comes with it. Docker Desktop's network translation makes every caller look like
the same address, so `KOPPEL_UI_ALLOW_FROM` cannot tell them apart, and one caller on the LAN typing wrong
passwords locks the admin out for a while. To keep the UI on this machine, give the network surface
its own port: set `KOPPEL_NETWORK_PORT: "5241"` and publish `127.0.0.1:5240:5240` and `5241:5241`.

**Close usbipd-win to the network.** Its installer adds a firewall rule named `usbipd` that lets
every machine on the local subnet connect to port 3240. That is plain USB/IP with no login, so
anyone on the subnet could attach a shared device without going through Koppel. Look at the rule:

```powershell
Get-NetFirewallRule -DisplayName usbipd | Get-NetFirewallAddressFilter
```

Then turn it off with `Disable-NetFirewallRule -DisplayName usbipd`, and check that the Koppel
devices page still lists your devices. If it says it cannot reach the USB/IP server, turn the
rule back on and limit `-RemoteAddress` to the Docker Desktop network with `Set-NetFirewallRule`.
Not tested: which of the two Docker Desktop needs.

## Windows client

1. Install usbip-win2 from its releases page, https://github.com/vadimgrn/usbip-win2/releases.
   It needs Windows 10 x64 1903 or later, or Windows 11 ARM64. It installs `usbip.exe` in
   `C:\Program Files\USBip`.

2. Start Koppel:

   ```powershell
   docker compose -f src\installer\docker\docker-compose.windows-client.yml up -d
   ```

3. Open https://localhost:5240/, accept the certificate warning, finish setup as a client and connect it to the server (server
   URL, fingerprint, client ID and secret, from the server's Clients page).

4. Attach a device. The device's details in Koppel show this command, ready to copy:

   ```powershell
   & "C:\Program Files\USBip\usbip.exe" attach -r 127.0.0.1 -b 3-2
   ```

   Koppel shows it as attached through the gateway. Detach it with the Detach button in Koppel, or
   with `usbip.exe detach -p <port>` using the port the attach printed. To see what the server
   shares: `usbip.exe list -r 127.0.0.1`.

The gateway is plain USB/IP with no login: whoever reaches it uses this client's access to the
server. The compose file publishes it on the host's `127.0.0.1:3240` only. Other containers on the
same Docker network can still reach it on port 3242.

usbip-win2 has no documented option for another port, so the gateway takes 3240 on the host. A
machine that also runs usbipd-win cannot use both at once.

## Signing in

The UI asks for a password. The first time, whoever opens the UI sets it, within 10 minutes of
Koppel starting (`KOPPEL_UI_SETUP_WINDOW_MINUTES`). After that the UI says setup has timed out: restart Koppel
(**Restart Koppel** in the menu of the tray icon, `Restart-Service Koppel` from an elevated prompt, or `docker compose restart` for Docker Desktop) and
open it again. (A Docker Desktop install runs the Linux image, so while `KOPPEL_UI_ALLOW_FROM` is not `local` its first password also needs the setup token from `docker compose logs koppel`; the native app needs none.)

- Native install: the UI answers this machine only, so only someone at this machine can set it.
- Docker Desktop: every caller looks like the same address, so `KOPPEL_UI_ALLOW_FROM` cannot keep the
  network out. With the server compose file, which publishes 5240 on every address, set the password
  right after the first start, or publish the UI on `127.0.0.1` only as described above.

The sign-in lasts 12 hours of use. After five wrong tries the UI locks that caller for a minute, and
the lock doubles with each further failure, up to an hour. If you forget the password, reset it, then
set a new one in the UI within 10 minutes:

```powershell
# native install, elevated prompt
& "C:\Program Files\Matumi\Koppel\Koppel.exe" reset-password

# Docker Desktop
docker compose run --rm koppel reset-password
```

## The native install

`Koppel-Setup.exe` comes with each [release](https://github.com/matumi-software/koppel/releases/latest). It installs Koppel as a Windows
service (LocalSystem, starts at boot, restarts after a crash or an error stop), the usbip-win2 driver, and the Visual C++
runtime, which usbip-win2's tools need (Koppel's own programs do not). From 0.2.0 it installs the Rust build: `Koppel.exe`, the service, and
`KoppelDesktop.exe`, the tray icon and the Start menu entry, about 11 MB in all. The service logs warnings, errors, and its
starts and stops with their reason, to the Application event log under the source Koppel, and everything at Info and above to `%ProgramData%\Koppel\logs\koppel.log` (5 MiB, five files). `Koppel.exe diagnostics`, run elevated, writes a report to attach to a support request. It adds an inbound firewall rule for `Koppel.exe` (TCP and UDP, any port, so it
follows a changed `Ports` setting) on Private and Domain networks, never Public. It is removed on uninstall and kept on repair and
upgrade. The UI is at https://localhost:5240 and answers on
this machine only: the installer sets `KOPPEL_UI_ALLOW_FROM=local` for the service. A native Windows server
is reached on the same port, 5240, through that rule. A network Windows has marked Public gets no rule: change it to Private, or add one by hand.

- **Tray icon:** while the service runs, every signed-in user gets a Koppel icon in the notification area. Windows 11 puts new icons in the overflow first; drag it onto the taskbar to keep it in sight. Its tooltip says how Koppel is doing, such as "linked to lab-pc, 2 devices attached" or "server, 3 devices shared, 1 in use", and an amber dot on the icon means something needs you: the link to the server is down, sharing is not available, or setup is waiting or has timed out. It also shows Windows notifications: on a server when a client wants to pair (click it to open Clients), and on a client when the link to the server drops and when it is back (once each, not on every retry) and when a device does not attach. Windows holds them back during Do not disturb. Each kind can be turned off under Configuration, Notifications, which only the Windows app shows. Click it, or pick **Open Koppel**, to open the UI in your browser. **Restart Koppel** asks first and then restarts the service, which also opens setup again when no password is set yet. **Shut down Koppel** asks first and then stops the service: shared and attached devices stop working, and Koppel stays off until the next boot or until you start it again from **Koppel** in the Start menu. Any signed-in user can do this, not only administrators. The icon goes when the service stops and comes back when it starts.
- **Start menu:** **Koppel** in the Start menu opens the UI in your browser. If Koppel is not running, it offers to start it, which takes an administrator (a UAC prompt), waits for the UI to answer and then opens it. It opens the address the service listens on, which the service writes to `HKLM\SOFTWARE\Matumi\Koppel` as it starts.
- **Finishing setup:** after an install or an upgrade, the last page of setup has **Launch**, which opens the UI the same way, so the first password can be set straight away. A quiet or passive install opens nothing.
- **Data folder:** `%ProgramData%\Koppel` is created with Administrators as owner and access for SYSTEM and Administrators only. If it already exists as a file or a link, or is owned by someone who is not an administrator, the install stops and says so: rename or delete the folder (copy any keys out first) and run setup again. A folder owned by an administrator, such as one an earlier install left, is taken over: the owner and access are reset for everything under it any link inside is deleted, and a file or folder inside that is not owned by an administrator is renamed aside (`not-admin-owned-<time>`) and not used. The service also locks `%ProgramData%\Koppel` itself down on every start.
- **Driver:** its output goes to the MSI log, not to a file in the data folder. Keep a log of the install to read it (for example `msiexec /i Koppel.msi /l*v setup.log`). If Windows says a restart finishes the driver install or its removal, setup or uninstall asks for one.
- **A usbip-win2 you installed yourself** is found and used as it is. Koppel never changes or removes its driver. Koppel's own uninstall removes only the driver it installed itself (recorded in `%ProgramData%\Koppel\driver-state.json`), and only a package Windows still lists as one of usbip-win2's (`usbip2_*.inf`), because Windows reuses the `oemNN.inf` numbers. A usbip-win2 installed on its own is looked for first, even when a record exists. If the record is missing, or is not owned and protected by administrators, the uninstall leaves the driver and prints how to remove it by hand.
- **The gateway is on by default** (127.0.0.1, port 3242), and serves only the Windows kernel, which is how usbip-win2's driver connects. Another program on the machine that connects to it is turned away, and so is any import Koppel did not ask for itself, such as another user's `usbip attach`. Koppel attaches devices through it by itself; you do not run `usbip attach` by hand.
- **Servers:** a native Windows machine can be a client. As a server it still needs `KOPPEL_USB_UPSTREAM` pointed at usbipd-win.

The elevated parts of the installer were run on a clean Windows 11 22H2 VM (see "What is tested"); the rollback of a failed fresh install was not exercised.
The installer is not code signed.

## Finding servers

Koppel finds servers on the local network and pairs with a code typed on the server (see
"Finding servers and pairing" in `docs/skakel.md`). In a container on Windows that does not
work: Docker Desktop's network does not pass broadcasts to or from the LAN. Enter the server's
address by hand there, or use the native install, which finds servers.

## Configuration

| Setting | Where | What |
|---|---|---|
| `KOPPEL_USB_UPSTREAM` | server | `host:port` of a USB/IP server to share from, instead of this kernel |
| `KOPPEL_GATEWAY_PORT` | client | Port of the plain USB/IP gateway. 3242 on native Windows, off elsewhere unless set. The Windows client compose file sets it |
| `KOPPEL_GATEWAY_ADDRESS` | client | Address it listens on. `127.0.0.1` unless set. The compose file sets `0.0.0.0` inside the container |
| `KOPPEL_USB_ATTACH_TIMEOUT_SECONDS` | client | How long an attach may take before it fails with a readable error. 20 |
| `KOPPEL_UI_ALLOW_FROM` | both | Who may use the UI: `local`, `lan` or `any`. `lan` unless set; the native installer sets `local` |
| `KOPPEL_UI_SETUP_WINDOW_MINUTES` | both | How long after a start, or a password reset, the first password may be set. 10 |
| `KOPPEL_NETWORK_PORT` | both | A port of its own for what other Koppel machines do. The UI's port (5240) unless set |

In a compose file these are environment variables, such as `KOPPEL_USB_UPSTREAM`. 0.1.x's names (`Usb__Upstream`) still work and are logged as deprecated.

## What is tested

- Tested with fakes in `koppel-usb`'s tests: a Koppel server sharing from a stand-in usbipd-win, a
  client attaching through it with traffic reaching the device, and a stand-in usbip-win2 attaching
  through the gateway, which refuses an import Koppel did not ask for.
- Real hardware: a Windows 11 client attached a Logitech USB receiver from a Linux server through usbip-win2, both with the native install and through Docker Desktop's gateway, and it worked as keyboard and mouse.
- Real hardware (2026-11-03): a native Windows 11 client attached two Logitech receivers and a flash drive from a Debian 13 server, across an installer upgrade. A file on the drive matched its checksum after a detach and reattach, and a 75 second network drop was recovered from by itself.
- Real usbipd-win (2026-11-03): a Windows 11 PC with usbipd-win 5.x shared a webcam to a Debian client, which streamed it at 30 fps. While another program on Windows held the camera open, usbipd-win could not export it and Koppel kept retrying; it attached once the program let go.
- Installer (2026-11-03), on a clean Windows 11 22H2 VM: fresh install, service recovery, major upgrade, uninstall, taking over an administrator-owned data folder and refusing one owned by a non-administrator. The rollback of a failed fresh install was not exercised.
- Tray icon (2026-11-08), installed over 0.1.1 on Windows 11: the icon appears for the signed-in user, a click opens the UI in their browser, Shut down Koppel stops the service without a recovery restart and the icon goes with it, the icon comes back when the service starts, and it comes back after Explorer restarts.
- 0.1.2 Windows app (2026-11-09), on the same PC: the Start menu entry opened the UI with Koppel running, and with it shut down it offered to start it, started it after the UAC prompt and then opened the UI. Restart Koppel brought the service back in 3 seconds, with the reason in the event log, and the tray came back. The tooltip showed the real link and device count. Configuration showed the Notifications panel for a client. Launch showed on the installer's last page after an upgrade from 0.1.1 and opened the UI. The notifications themselves were tried with a test tray fed the same messages, not with real pairing or link events.
- Rust Windows app (2026-11-09, not installed yet): the tray run by hand with its standard input fed the service's lines showed the tooltip, the amber dot, the menu and a notification, skipped lines it could not read, and removed its icon and exited when the input closed. The launcher's pipe started the real tray in the same session (the service's token path needs LocalSystem). The Start menu entry, pointed at a Rust Koppel on another port, found `/healthz` answering and opened the browser. The installer builds. Not tried until it is installed: the service itself, the tray in each session as the user, Shut down and Restart, the `UiUrl` value, the event log entries, the upgrade from 0.1.2, Launch, and the icon coming back after Explorer restarts.
- The Windows client through Docker Desktop was re-run on 2026-11-03: attach through the gateway without elevation, and Detach from Koppel.
- Not tested: printers, the attach deadline on its own, and a webcam from a Linux server to a Windows client, which attaches but delivers no frames (#22).
