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

A Linux machine needs none of this. It runs the normal image (`src/Koppel/deploy`), and the mixes
work: a Windows client with a Linux server, and the other way round.

## Build the image

From `src\Koppel` in the repo:

```powershell
docker compose build
```

That builds `matumisoftware/koppel:0.1.0` (the tag in `KOPPEL_TAG`), which both Windows compose files use. The compose files name that tag, so a build made here is used as it is. To use the published image instead, skip this step and Docker pulls it.

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
   docker compose -f src\Koppel\deploy\docker-compose.windows-server.yml up -d
   ```

4. Open http://localhost:5240/ and finish setup as a server. The devices page shows what
   usbipd-win shares. Share and unshare with `usbipd bind` and `usbipd unbind`; the switch in
   Koppel cannot change it. USB/IP sends no product names, so devices show by VID:PID.

Koppel's USB port on this machine is 3241, because usbipd-win holds 3240. Clients learn the port
from the server, so nothing changes on their side.

**Close usbipd-win to the network.** Its installer adds a firewall rule named `usbipd` that lets
every machine on the local subnet connect to port 3240. That is plain USB/IP with no login, so
anyone on the subnet could attach a shared device without going through Koppel. Look at the rule:

```powershell
Get-NetFirewallRule -DisplayName usbipd | Get-NetFirewallAddressFilter
```

Then turn it off with `Disable-NetFirewallRule -DisplayName usbipd`, and check that the Koppel
devices page still lists your devices. If it says it cannot reach the USB/IP server, turn the
rule back on and limit `-RemoteAddress` to the Docker Desktop network with `Set-NetFirewallRule`.
Not tested yet: which of the two Docker Desktop needs.

## Windows client

1. Install usbip-win2 from its releases page, https://github.com/vadimgrn/usbip-win2/releases.
   It needs Windows 10 x64 1903 or later, or Windows 11 ARM64. It installs `usbip.exe` in
   `C:\Program Files\USBip`.

2. Start Koppel:

   ```powershell
   docker compose -f src\Koppel\deploy\docker-compose.windows-client.yml up -d
   ```

3. Open http://localhost:5240/, finish setup as a client and connect it to the server (server
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

The UI asks for a password. The first time, it also asks for a one-time setup code, so only someone
with access to this machine can set the first password.

- Docker Desktop: `docker logs <container>` shows `Koppel setup code: XXXX-XXXX`.
- Native install: open `%ProgramData%\Koppel\config\setup-code.txt` (an administrator can read it).

The sign-in lasts 12 hours of use. After five wrong tries the UI locks that caller for a minute, and
the lock doubles with each further failure, up to an hour. If you forget the password, reset it and
read a new setup code:

```powershell
# native install, elevated prompt
& "C:\Program Files\Matumi\Koppel\Koppel.exe" --reset-password

# Docker Desktop
docker compose exec koppel dotnet Koppel.dll --reset-password
```

## The native install

`src\Koppel\installer\build.ps1` makes `bin\Koppel-Setup.exe`. It installs Koppel as a Windows
service (LocalSystem, starts at boot, restarts after a crash), the usbip-win2 driver, the Visual C++
runtime and a firewall rule for finding servers. The UI is at http://localhost:5240 and answers on
this machine only: the installer sets `Ports__HttpAddress=127.0.0.1` for the service.

- **Data folder:** `%ProgramData%\Koppel` is created with Administrators as owner and access for SYSTEM and Administrators only. If it already exists as a file or a link, or is owned by someone who is not an administrator, the install stops and says so: rename or delete the folder (copy any keys out first) and run setup again. A folder owned by an administrator, such as one an earlier install left, is taken over: the owner and access are reset for everything under it and any link inside is deleted.
- **Driver:** its output goes to the MSI log, not to a file in the data folder. Keep a log of the install to read it (for example `msiexec /i Koppel.msi /l*v setup.log`). If Windows says a restart finishes the driver install, setup asks for one.
- **A usbip-win2 you installed yourself** is found and used as it is. Koppel never changes or removes its driver. Koppel's own uninstall removes only the driver it installed itself (recorded in `%ProgramData%\Koppel\driver-state.json`). If the record is missing, the uninstall leaves the driver and prints how to remove it by hand.
- **The gateway is on by default** (127.0.0.1, port 3242), and serves only the Windows kernel, which is how usbip-win2's driver connects. Another program on the machine that connects to it is turned away. Koppel attaches devices through it by itself; you do not run `usbip attach` by hand.
- **Servers:** a native Windows machine can be a client. As a server it still needs `Usb:Upstream` pointed at usbipd-win.

Not tested: the elevated parts of the installer (taking over an existing data folder, the driver
record and standalone detection, rollback, the restart request) have not been run on a test machine.
The installer is not code signed.

## Finding servers

Koppel finds servers on the local network and pairs with a code typed on the server (see
"Finding servers and pairing" in `docs/architecture.md`). In a container on Windows that does not
work: Docker Desktop's network does not pass multicast DNS to or from the LAN. Enter the server's
address by hand there, or use the native install, which finds servers and has its firewall rule.

## Configuration

| Setting | Where | What |
|---|---|---|
| `Usb:Upstream` | server | `host:port` of a USB/IP server to share from, instead of this kernel |
| `Usb:ListenPort` | server | The USB port a new install starts with. After that, the Machine page sets it |
| `Gateway:Port` | client | Port of the plain USB/IP gateway. 3242 on native Windows, off elsewhere unless set. The Windows client compose file sets it |
| `Gateway:Address` | client | Address it listens on. `127.0.0.1` unless set. The compose file sets `0.0.0.0` inside the container |
| `Usb:AttachTimeoutSeconds` | client | How long an attach may take before it fails with a readable error. 20 |
| `Ports:HttpAddress` | both | Address the UI port binds to. Any address unless set; the native installer sets `127.0.0.1` |

In a compose file these are environment variables with `__` for `:`, such as `Usb__Upstream`.

## What is tested

- Tested with fakes in `Koppel.Tests`: a Koppel server sharing from a stand-in usbipd-win, a client
  attaching through it, and a stand-in usbip-win2 listing and attaching through the gateway, with
  USB bytes going both ways through TLS.
- Real hardware: a Windows 11 client attached a Logitech USB receiver from a Linux server through usbip-win2, both with the native install and through Docker Desktop's gateway, and it worked as keyboard and mouse.
- Not tested yet: a real usbipd-win with a real device, bulk transfers (storage, printers), and the changes made after those hardware runs (attach deadline, the kernel-only gateway check, detach on stop), which are covered by fakes only.
