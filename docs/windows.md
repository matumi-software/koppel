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

That builds `matumi/koppel:latest`, which both Windows compose files use.

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
| `Gateway:Port` | client | Port of the plain USB/IP gateway. Off when unset |
| `Gateway:Address` | client | Address it listens on. `127.0.0.1` unless set |

In a compose file these are environment variables with `__` for `:`, such as `Usb__Upstream`.

## What is tested

- Tested with fakes in `Koppel.Tests`: a Koppel server sharing from a stand-in usbipd-win, a client
  attaching through it, and a stand-in usbip-win2 listing and attaching through the gateway, with
  USB bytes going both ways through TLS.
- Not tested yet: real usbipd-win and real usbip-win2 with a real device.
