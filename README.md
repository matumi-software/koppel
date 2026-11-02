# Koppel

Share USB devices across the network. Plug a device into one machine and use it on another.

Koppel speaks the standard USB/IP protocol and adds a web UI, authentication and encrypted transport
around it. A machine runs as a server (shares its devices), a client (attaches them) or both. Clients
find servers on the local network and pair by typing a code, so nobody copies secrets around.

Status: early development. A Linux server in Docker shares a real USB keyboard and mouse receiver
to a Windows 11 client through the installed service. Storage, printers, keyboards and mice are the
devices we aim to support first. Work happens on `dev`; `main` holds releases.

| Platform | Server | Client |
|---|---|---|
| Linux (Docker) | Yes | Yes |
| Windows (installer) | Not yet | Yes, with usbip-win2's driver bundled |
| Windows (Docker Desktop) | Through usbipd-win, see [docs/windows.md](docs/windows.md) | Through usbip-win2, see [docs/windows.md](docs/windows.md) |

## Run it on Linux

The image is `matumisoftware/koppel` on Docker Hub (amd64 for now).

1. Copy [src/Koppel/deploy/prepare-host.sh](src/Koppel/deploy/prepare-host.sh) to the machine and
   run it once. It installs Docker and loads the kernel's USB/IP modules now and at every boot:

   ```sh
   sudo sh prepare-host.sh
   ```

2. Copy [src/Koppel/deploy/docker-compose.yml](src/Koppel/deploy/docker-compose.yml) to the machine
   and start Koppel next to it:

   ```sh
   docker compose up -d
   ```

3. Open `http://<machine>:5240` and choose the role.

The container uses the host's network so clients can find it. It restarts after a crash, and comes
back at the next reboot even if someone stopped it by hand.

To upgrade:

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
driver, the Visual C++ runtime, and a firewall rule that lets Koppel find servers. Open
http://localhost:5240 afterwards.

## Pair a client

1. On the client, open Configuration. Servers on your network lists the Koppel servers it hears.
   Press Pair, and the client shows a six-digit code.
2. On the server, open Clients. Under Waiting to pair, type that code and press Approve.

The client then has its own credentials and connects. A server on another network is entered by
address under Configuration instead.

## Ports

| Port | What |
|---|---|
| 5240 TCP | Web UI (HTTP) |
| 5241 TCP | Token endpoint, pairing and the API other Koppel machines call (HTTPS) |
| 3240 TCP | USB traffic (TLS) |
| 5353 UDP | Finding servers (multicast DNS) |

The UI on port 5240 has no login yet. Anyone who can reach it can manage the machine, so keep it on
a trusted network.

## Develop

See the commands in [CLAUDE.md](CLAUDE.md) and the design in
[docs/architecture.md](docs/architecture.md).

## License

MIT, see [LICENSE](LICENSE). The Windows installer bundles usbip-win2 (BSD-2-Clause); its notices
ship as `THIRD-PARTY-NOTICES.txt`.
