# Koppel

Share USB devices across the network. Plug a device into one machine and use it on another.

Koppel speaks the standard USB/IP protocol and adds a web UI, authentication and encrypted transport
around it. A machine runs as a server (shares its devices), a client (attaches them) or both. Clients
find servers on the local network and pair by typing a code, so nobody copies secrets around.

Status: version 0.1.0, early. On real hardware, a Debian 13 server shares USB receivers and a flash drive to a
Windows 11 client, and a Windows server through usbipd-win shares a webcam to a Linux client. Printers are
untried, webcams do not stream from a Linux server to a Windows client yet, and the arm64 image and `.deb`
are untested on a Raspberry Pi. See [CHANGELOG.md](CHANGELOG.md). Work happens on short-lived branches merged into `main` through pull requests; releases are tags on `main`.

| Platform | Server | Client |
|---|---|---|
| Linux (Docker) | Yes | Yes |
| Linux (.deb, no Docker) | Yes | Yes |
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

3. Within 10 minutes, open `https://<machine>:5240`, set the password and choose the role. Whoever
   opens the UI first sets the password, so do it straight away. If the 10 minutes pass, the UI says
   so: run `docker compose restart` and open it again. The browser warns about the certificate the
   first time, because it is Koppel's own; accept it to go on.

The container uses the host's network so clients can find it. It restarts after a crash, and comes
back at the next reboot even if someone stopped it by hand. The image has a health check, but plain Docker only reports an unhealthy container and does not restart it.

The compose file pins the image version (`matumisoftware/koppel:0.1.0`). To upgrade, change the tag to the new
version, then:

```sh
docker compose pull && docker compose up -d
```

Settings, the server's certificate, registered clients and the encryption keys live in the `koppel`
volume, not in the image, so an upgrade keeps them.

### Without Docker

A `.deb` package runs Koppel as a systemd service on Debian 12 and 13, Ubuntu 22.04 and later, and
64-bit Raspberry Pi OS, for amd64 and arm64. It needs no .NET and no Docker. Download the package for
the machine from the [latest release](https://github.com/matumi-software/koppel/releases/latest) and
install it:

```sh
wget https://github.com/matumi-software/koppel/releases/download/v0.1.0/koppel_0.1.0_amd64.deb
sudo apt install ./koppel_0.1.0_amd64.deb
```

On a Raspberry Pi, take `koppel_0.1.0_arm64.deb` instead.

The install loads the USB/IP modules now and at every boot, then starts the `koppel` service. Open
`https://<machine>:5240` within 10 minutes and set the password (after that, `sudo systemctl restart koppel`
opens setup again). If a module will not load, the install says so; the fix is the same as above.

- Config and keys live in `/var/lib/koppel`. An upgrade (`sudo apt install` of a newer `.deb`) and
  `sudo apt remove koppel` keep them; `sudo apt purge koppel` deletes them.
- Settings go in `/etc/default/koppel` as environment variables, one per line, e.g. `Ui__AllowFrom=lan`.
  Then `sudo systemctl restart koppel`.
- The service runs as root, as the container does, and restarts if it stops.

## Run it on Windows

Download `Koppel-Setup.exe` from the [latest release](https://github.com/matumi-software/koppel/releases/latest)
and run it. It installs Koppel as a Windows service, usbip-win2's signed
driver and the Visual C++ runtime. It also adds an inbound firewall rule for the Koppel program on Private and Domain networks, so a Windows server can be reached without one added by hand. Open
https://localhost:5240 afterwards, accept the browser's certificate warning and set the password within 10 minutes
(after that, restart the Koppel service). The UI answers on this machine only. The Koppel icon in the notification
area opens the UI, and its menu shuts Koppel down until the next boot. See
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

Koppel needs only 5240, TCP and UDP. `Ports:Network` puts what other machines do on a port of its own,
and `Discovery:Port` moves discovery; see [docs/architecture.md](docs/architecture.md).

A Windows client also has a port that never leaves the machine: a gateway on 127.0.0.1:3242 for
usbip-win2's driver. The driver speaks plain USB/IP and cannot sign in, so it connects there and Koppel
carries each connection to the server over TLS on 5240. Through Docker Desktop the gateway is published on
the host as 127.0.0.1:3240 instead. See [docs/windows.md](docs/windows.md).

The browser warns about Koppel's certificate the first time, because it is Koppel's own. On a network you do not
trust, check that its SHA-256 fingerprint is the one under Clients before you accept it. If you forget the password:

```sh
docker compose exec koppel dotnet Koppel.dll --reset-password
```

With the Linux package, run `sudo koppel --reset-password`. On Windows, from an elevated prompt, run `Koppel.exe --reset-password` from `C:\Program Files\Matumi\Koppel`. Then open
the UI within 10 minutes and set a new password.

## Help test Koppel

Koppel is young, and every report helps. The most useful things to send:

- **Which devices work.** Tried a device, whether it worked or not? Open a
  [device report](https://github.com/matumi-software/koppel/issues/new?template=device-report.yml) with its
  VID:PID (Koppel's device list shows it) and what runs on each end.
- **Bugs.** Open a [bug report](https://github.com/matumi-software/koppel/issues/new?template=bug-report.yml)
  with the steps and the logs: `docker logs <container>` in Docker, `sudo journalctl -u koppel` for the Linux
  package, or Event Viewer (Windows Logs, Application, source Koppel) on Windows. The Activity page in the UI
  shows what happened too.
- **Ideas and questions.** Bring them to [Discussions](https://github.com/matumi-software/koppel/discussions).

Security problems go privately through [SECURITY.md](SECURITY.md), not into an issue.

## Develop

See the commands in [CLAUDE.md](CLAUDE.md) and the design in
[docs/architecture.md](docs/architecture.md). To build the Windows installer, the Linux packages or the
image yourself, see "Building and releasing" there; to run your own build on another machine, see
"Running on another machine".

## License

Copyright (C) 2026 Matumi Software. Koppel is free software under the GNU General Public License,
version 3 or (at your option) any later version (GPL-3.0-or-later), see [LICENSE](LICENSE).
Releases up to and including 0.1.0 were published under the MIT licence and stay under it.

Koppel's transport between machines is [Skakel](https://github.com/matumi-software/skakel) (MIT), a
NuGet package; its notice is in `THIRD-PARTY-NOTICES.txt` with the other packages'.

The Windows installer bundles usbip-win2 (BSD-2-Clause); its notices ship as `THIRD-PARTY-NOTICES.txt`,
which the setup window's licence link opens. The setup window's theme files
(`installer/Koppel.Setup/KoppelTheme.*`) are adapted from WiX and stay under the Microsoft Reciprocal
License; they only style the installer and are not part of Koppel.
