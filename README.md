# Koppel

Share USB devices across the network. Plug a device into one machine and use it on another.

Koppel speaks the standard USB/IP protocol and adds a web UI, authentication and encrypted transport
around it. A machine runs as a server (shares its devices), a client (attaches them) or both. Clients
find servers on the local network and pair by typing a code, so nobody copies secrets around.

Status: version 0.1.0, early. A Linux server in Docker shares a real USB keyboard and mouse receiver
to a Windows 11 client through the installed service. Storage and printers are the devices we aim to support
next, and are untried on real hardware. The newest USB changes are covered by tests with fakes and not yet
re-run on real hardware. See [CHANGELOG.md](CHANGELOG.md). Work happens on `dev`; `main` holds releases.

| Platform | Server | Client |
|---|---|---|
| Linux (Docker) | Yes | Yes |
| Windows (installer) | Not yet | Yes, with usbip-win2's driver bundled |
| Windows (Docker Desktop) | Through usbipd-win, see [docs/windows.md](docs/windows.md) | Through usbip-win2, see [docs/windows.md](docs/windows.md) |

## Run it on Linux

The image is `matumisoftware/koppel` on Docker Hub, for amd64 and arm64. A Raspberry Pi needs the 64-bit
Raspberry Pi OS, which the script below treats as Debian.

1. Copy [src/Koppel/deploy/prepare-host.sh](src/Koppel/deploy/prepare-host.sh) to the machine and
   run it once. It installs Docker from Docker's apt repository (Debian and Ubuntu) and loads the kernel's USB/IP
   modules now and at every boot:

   ```sh
   sudo bash prepare-host.sh
   ```

   If a module will not load, the script says so; on an Ubuntu cloud kernel the fix is usually
   `sudo apt-get install linux-modules-extra-$(uname -r)`.

2. Copy [src/Koppel/deploy/docker-compose.yml](src/Koppel/deploy/docker-compose.yml) to the machine
   and start Koppel next to it:

   ```sh
   docker compose up -d
   ```

3. Find the one-time setup code with `docker logs <container>` (it reads `Koppel setup code: XXXX-XXXX`), open
   `https://<machine>:5240`, set the password and choose the role. The browser warns about the
   certificate the first time, because it is Koppel's own; accept it to go on.

The container uses the host's network so clients can find it. It restarts after a crash, and comes
back at the next reboot even if someone stopped it by hand. The image has a health check, but plain Docker only reports an unhealthy container and does not restart it.

The compose file pins the image version (`matumisoftware/koppel:0.1.0`). To upgrade, change the tag to the new
version, then:

```sh
docker compose pull && docker compose up -d
```

Settings, the server's certificate, registered clients and the encryption keys live in the `koppel`
volume, not in the image, so an upgrade keeps them.

To run your own build on another machine instead, see "Running on another machine" in
[docs/architecture.md](docs/architecture.md).

## Run it on Windows

Build the installer from `src/Koppel/installer`:

```powershell
.\build.ps1
```

That makes `bin\Koppel-Setup.exe`. It installs Koppel as a Windows service, usbip-win2's signed
driver and the Visual C++ runtime. It also adds an inbound firewall rule for the Koppel program on Private and Domain networks, so a Windows server can be reached without one added by hand. Open
https://localhost:5240 afterwards and accept the browser's certificate warning. The first time, it asks for a setup code: it is in
`%ProgramData%\Koppel\config\setup-code.txt`. The UI answers on this machine only. See
[docs/windows.md](docs/windows.md) for what the installer does to `%ProgramData%\Koppel` and the driver.

## Pair a client

1. On the client, open Configuration. Servers on your network lists the Koppel servers it hears.
   Press Pair, and the client shows a six-digit code.
2. On the server, open Clients. Under Waiting to pair, type that code and press Approve.

The client then has its own credentials and connects. A server on another network is entered by
address under Configuration instead.

## Ports

| Port | What |
|---|---|
| 5240 TCP | The web UI, and everything other Koppel machines do: tokens, pairing, the device list and USB traffic. HTTPS with the server's own certificate. The UI needs the password and answers the local network only, this machine only on native Windows (`Ui:AllowFrom`) |
| 5240 UDP | Finding servers on the local network |
| 3242 TCP | Gateway for a USB/IP driver on the same machine (plain, no login). Native Windows client: on, 127.0.0.1 only. Windows Docker client: inside the container, published as 127.0.0.1:3240 on the host |

Koppel needs only 5240, TCP and UDP. `Ports:Network` puts what other machines do on a port of its own,
and `Discovery:Port` moves discovery; see [docs/architecture.md](docs/architecture.md).

The browser warns about Koppel's certificate the first time, because it is Koppel's own. On a network you do not
trust, check that its SHA-256 fingerprint is the one under Clients before you accept it. If you forget the password:

```sh
docker compose exec koppel dotnet Koppel.dll --reset-password
```

On Windows, from an elevated prompt, run `Koppel.exe --reset-password` from `C:\Program Files\Matumi\Koppel`. Then read
a new setup code and set a new password.

## Develop

See the commands in [CLAUDE.md](CLAUDE.md) and the design in
[docs/architecture.md](docs/architecture.md).

## License

MIT, see [LICENSE](LICENSE). The Windows installer bundles usbip-win2 (BSD-2-Clause); its notices
ship as `THIRD-PARTY-NOTICES.txt`, which the setup window's licence link opens.
