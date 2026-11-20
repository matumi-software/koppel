# Set up Koppel, step by step

This guide takes two machines on the same network from nothing to a USB device shared from one and used on the
other. The pictures it refers to are in the [README](../README.md).

You need a **server**, where the USB devices are plugged in, and a **client**, where you want to use them. One
machine can be both. Both need Koppel installed; the install steps are in the README:
[Run it on Linux](../README.md#run-it-on-linux) and [Run it on Windows](../README.md#run-it-on-windows).

## 1. Install Koppel on the server

Install it as the README says for the machine, then open its UI: `https://<server>:5240` (on Windows,
`https://localhost:5240`). The browser warns about the certificate the first time, because it is Koppel's own;
accept it to go on.

Set the admin password within 10 minutes. On Linux and in Docker the first-run screen asks for a **setup token**
that Koppel prints in its log (`docker compose logs koppel`, or `sudo journalctl -u koppel`); it works once. Then
choose what the computer does: **server**, **client** or **both**. Choose **server** (or **both**) here.

## 2. Install Koppel on the client

Install it the same way. On Windows the installer includes the USB driver, so nothing else is needed. Open its UI,
set a password there too (it can be the same one), and choose **client** (or **both**).

## 3. Share a device on the server

On the server, open **Devices**. Every plugged-in device has a **Share** switch. Turn it on for each device other
machines may use. A shared device that nobody uses shows **Shared**; one in use shows **Attached**.

## 4. Link the client to the server

The client has to be linked to the server once. There are two ways, and neither needs anything approved on the
server.

**With a pairing code.** On the server, open **Clients**. **New pairing code** has a **Show** button: it shows
eight digits that work once, for 60 seconds.

**With the server key.** On the same page, **Server key** has a **Show** button too. The key links a computer at
any time without a code. Copy it, and keep it private.

Then, on the client, open **Connection** and press the **+** button. Type the server's address with `https://` in
front, such as `https://192.168.1.20:5240`, and the pairing code, or choose **Use the server key** and paste the
key. Press **Connect** (it reads **Add server** once the client has one).

The client now has its own credentials and links to the server. Add as many servers as you like. Each one is listed
under **Connection** with its link state, and on Connection a server's **Reconnect**, **Retry every** and
**Give up after** can be changed, and a server can be tested or removed (which releases its devices).

## 5. Attach a device on the client

On the client, open **Remote devices**. The devices the servers share appear together there, each saying which
server it is on. Turn a device's switch on to attach it: it then behaves as if it were plugged into the client
(a flash drive gets a drive letter, a keyboard or a mouse works). Turn the switch off to give it back.

## 6. If something does not work

- The strip at the top right says **Live** while the UI can reach its own Koppel service. If it says it is not
  connected, check that the service is running.
- **System check** says what a machine is missing: the USB/IP kernel modules on Linux, the driver on Windows.
- **Activity** lists what happened, and why a device could not be attached.
- The two machines must reach each other on port 5240, TCP and UDP (see [Ports](../README.md#ports)). On Windows the
  installer adds the firewall rule.
- A server that restarts with Docker comes back by itself: the compose file restarts it, but Docker itself has to
  start at boot (`sudo systemctl enable docker`). The USB/IP kernel modules have to load at boot too, which
  `prepare-host.sh` and the `.deb` set up.

## Known issues in 0.3.0

Fixes are planned for the next release.

- **Docker refuses to start on a new volume.** Koppel stops with "/koppel/config belongs to user 65532"
  ([#243](https://github.com/matumi-software/koppel/issues/243)). Run `docker compose stop`, then
  `docker run --rm -v koppel_koppel:/data alpine chown -R 0:0 /data`, then `docker compose up -d`. The volume is
  named `<folder>_koppel`; `docker volume ls` shows it.
- **The server address needs `https://`.** The "Found on this network" button fills the address in without it, and
  Koppel refuses it ([#244](https://github.com/matumi-software/koppel/issues/244)). Add `https://` by hand.
- **A device may not be shared again after a restart or a replug.** If a device is not shared after the server
  restarts, or after it was unplugged and plugged back in, turn **Share** back on
  ([#245](https://github.com/matumi-software/koppel/issues/245)).
- **After an upgrade a client can show its server offline.** If it does, add the server again with a new pairing
  code ([#246](https://github.com/matumi-software/koppel/issues/246)).
