# Skakel and Koppel

## What Skakel does and what Koppel does

Since 0.1.1 the transport between Koppel machines is Skakel, a library under the MIT licence. Its wire
format is protocol version 1, described in Skakel's [`docs/protocol.md`](https://github.com/matumi-software/skakel/blob/main/docs/protocol.md): discovery,
the info endpoint, pairing, tokens and the client generation, the certificate handover, following a
server that moved, the hub's own messages and the streams. Koppel 0.1.0 spoke what that document calls
version 0, so 0.1.0 and 0.1.1 cannot talk to each other. The Rust build (0.2.0) speaks protocol
version 2, Skakel 0.2.0's, which keeps discovery, info, pairing, tokens and the handover and moves the
hub and the streams to gRPC, so it and 0.1.x cannot talk to each other either (see "The roles on Skakel
in Rust").

What Koppel adds on top, and what the rest of this document is about:

- Where Skakel's state lives: `secrets.json` (server) and `connection.json` (client), encrypted (see ["Config on disk"](config.md#config-on-disk)).
- Which port serves Skakel's endpoints, and in which role (see ["Ports and what they serve"](ports.md#ports-and-what-they-serve)).
- The UI over Skakel's clients, keys and pairing requests, with the confirm rule.
- Koppel's own hub message, `DevicesChanged`, and its own stream type, `usb` (see ["USB data plane"](usb.md#usb-data-plane)).
- Answering discovery only while the role is server or both (see ["Finding servers and pairing"](#finding-servers-and-pairing)).
- On the client: what the link learns is saved, and the attached devices carry on through it.

## Finding servers and pairing

Discovery, following a server that moved, and the pairing exchange with its code, its poll token and its
limits are Skakel's; see "Discovery", "Following a server" and "Pairing" in Skakel's
[`docs/protocol.md`](https://github.com/matumi-software/skakel/blob/main/docs/protocol.md). What Koppel adds:

- **Discovery:** a client browses with skakel-client's `Browser` on its discovery port (`Discovery:Port`, the HTTPS port number 5240 unless changed), keeps only servers whose app id is `koppel`, and lists them under Configuration, Servers on your network, leaving out itself. When other addresses answer with the same fingerprint, the row says so: a machine on more than one network does that, and so does one copying another server's fingerprint. A server answers probes only while its role is server or both. Koppel answers them itself (`koppel-web/src/roles/skakel/announcer.rs`), in Skakel's format, so the answers follow the role. `Discovery:Enabled=false` turns both sides off. Broadcasts stay on the local network, so a server elsewhere is entered by address.
- **Why not multicast DNS:** it worked, but it needs UDP 5353 as well, and Koppel's aim is one port number between machines.
- **Following a moved server:** the link follows its server when the saved address stops answering and discovery hears it, with a certificate it trusts, at another address. Koppel saves the new address through `ConnectionStore` like any other write, unless the person saved another connection meanwhile, so the UI hears `ConnectionChanged` and the activity log gets a `serverMoved` entry. The attached devices carry on. With `Discovery:Enabled=false`, or a server outside broadcast reach, nothing moves.
- **Pairing:** instead of copying a client ID and secret, the client shows a six-digit code and a person types it on the server's Clients page, under Waiting to pair, as with Moonlight and Sunshine. On a match the server registers the client and Skakel hands it its credentials; they go straight into `connection.json`, and `ConnectionChanged` confirms it as for any saved connection. Approve answers 202 and is confirmed by `PairingRequestsChanged` and `ClientsChanged`; a wrong code is a 400 at once. Approvals and refusals go to the activity log.
- **Replacing a connection:** if the client already has a connection to a different server, an approved pairing does not overwrite it. The client goes to a `confirm` state that shows the new server's name and fingerprint, and the person chooses Replace or Discard (`POST /api/client/pairing/confirm` to replace, `DELETE /api/client/pairing` to discard). The held credentials stay in memory until the person acts, or pairing is started again or cancelled. The `paired` state clears once the saved connection no longer matches it.
- **Endpoints:** `/api/server/pairing` for the server's UI; `/api/client/discovery` and `/api/client/pairing` for the client's UI. Pushes: `ServersDiscovered`, `PairingStatusChanged`, `PairingRequestsChanged` (an object: `requests` and `budgetExhausted`, which says the server had its fill of requests for now and is turning new ones away; only requests that got as far as a code count). The client's pairing status carries the server's fingerprint, for the confirm step. A machine that is a client only answers 404 to everything, and the client says it may be set up as a client only.
- **Saved connection:** `PUT /api/client/connection` keeps the stored secret only when the server address and fingerprint are unchanged. Otherwise the request must carry a new secret, or it is refused with 400.
- **Docker:** broadcasts do not cross Docker's bridge network, so the Linux server compose file uses the host network. On Windows, Docker Desktop's host networking does not reach the LAN: use the native install there. On Windows the installer adds the firewall rules (see Firewall below).

### The roles on Skakel in Rust

The server and client roles run on Skakel 0.2.1 (protocol version 2, gRPC), taken as git
dependencies at the commit that let an app serve Skakel on its own listener (Skakel PR #43) until
the 0.2.0 release tag is published. The code is in `koppel-web/src/roles/skakel/`.

| Module | What |
|---|---|
| `store.rs` | `secrets.json` as skakel-server's store, and the certificate the web ports present |
| `server.rs` | `SkakelServerRole`: skakel-server mounted on the web ports, its clients, keys and pairing, and its events into the push and the activity log |
| `announcer.rs` | Discovery answers while the role is server or both |
| `remote.rs` | Koppel's own gRPC service, the remote device list, and `/api/remote/devices` |
| `streams.rs` | The `usb` stream type as koppel-usb's `DeviceStream`, both ends |
| `client.rs` | `SkakelClientRole`: the connection test, discovery and pairing |
| `links.rs` | The link to the saved server as koppel-usb's `Links`, and what it saves in `connection.json` |

- **Skakel on the web ports:** Skakel's server is started mounted (`start_mounted`): it binds no
  port of its own, and Koppel's web ports serve the UI, the plain HTTP redirect and the port and role
  rules on the same port as 0.1.x, with Skakel beside them. After the port and role check, the web
  ports hand Skakel's service what is Skakel's: the gRPC calls (told by their content type; network
  surface, server role), `/skakel/*` and `/api/remote/*`. Each request carries the caller's real
  address (`ConnectInfo`) from the accept loop. Skakel's calls run on the caller's own connection, so
  a client's USB streams keep a connection each and its calls end when it goes. Every connection is
  served with Skakel's connection settings (`connection_builder`): a ping after 15 seconds of quiet,
  dead 15 seconds later, flow-control windows that grow with the link, and 30 seconds for an
  HTTP/1.1 request's headers.
- **TLS:** while the role is server or both, each new connection to a web port shakes hands with
  Skakel's server's own TLS settings (`tls_config`), which ask the server for its current certificate
  on every handshake, so a switched certificate shows from the next connection and a prepared one
  never shows before the switch, with ALPN `h2` and `http/1.1` and no session resumption. In the
  client role the web ports present Koppel's own certificate slot, which a save of `secrets.json`
  keeps on the same certificate, with the same settings.
- **Limits:** Skakel keeps its own per-address limits, by the caller's real address:
  `RateLimits:TokenPerMinute` and `RateLimits:PairPerMinute`, 5 pairing requests opened in 5 minutes
  and 3 open at once. Since Skakel sits behind the port and role check, a call to the wrong port or
  role gets its 404 without counting, where 0.1.x counted it. The address a pairing request came
  from, in the server's UI and the activity log, is the one Skakel saw.
- **State:** `secrets.json` keeps 0.1.x's shape (Skakel 0.1's state, the certificates as PKCS#12),
  converted at each load and save, so the clients, their secret hashes and generations, the
  certificate and a prepared next one carry over unchanged. A signing key that is not ECDSA P-256
  (0.1.x's RSA one) is replaced at start; tokens last 30 minutes and live only in a client's memory.
  A save that changes the certificate replaces what the web ports present from the next connection.
- **Role change:** leaving the server role starts Skakel's server again, which ends every client's
  control call, remote device list and USB stream with `STOPPING`, as 0.1.x ended its hub
  connections and streams with close code 4003. The new server starts before the old one stops; from
  then on new requests go to the new server's service, and new connections shake hands with its TLS
  settings once the role is server or both again.
- **Pairing:** a request the server's person refused leaves the list at once, as in 0.1.x; Skakel
  0.2.0 shows refused requests for 15 seconds, which Koppel keeps only for three wrong codes.
  Approvals and refusals go to the activity log as `clientAdded`, as in 0.1.x.
- **Discovery:** Koppel answers probes itself while the role is server or both, on `Discovery:Port`,
  bound to `Ports:Address` when that is an IPv4 address (a server on loopback has nobody else to
  answer) and to every IPv4 address otherwise. The client browses with skakel-client's `Browser`
  while the role is client or both, leaves out its own certificate, and lists servers as 0.1.x did.
- **Remote device list:** Koppel's own gRPC service `koppel.remote.v1.RemoteDevices`
  (`koppel-web/proto/koppel/remote/v1/devices.proto`), beside Skakel's on the same port and
  connection. `Watch` sends the remote view (the shared devices, without who holds them) at once and
  again on each change, in place of 0.1.x's hub message `DevicesChanged`. The client calls it again
  each time its link is connected after it was not, and after a list that ended while the link stayed
  up. `GET /api/remote/devices` with a client's token still answers the same view.
- **USB streams:** Koppel's stream type `usb` on Skakel's `StreamService`, with no target and no
  control messages. Its data frames carry USB/IP as 0.1.x's WebSocket binary frames did, and each
  stream runs on a connection of its own. A stream the data plane ends normally, or because the role
  left the server side, ends with `OK` (leaving the role ends them with `STOPPING` anyway); one it
  ends after a failure ends with `ABORTED`, domain `skakel.streams.usb`, reason `FAILED`.
- **The link:** koppel-usb's client makes it through `SkakelLinks` under its own lock, so making
  one only builds skakel-client's `Link`; it starts in `Link::start`. Its states become koppel-usb's
  link states with Skakel's fixed wording, so a revoke, a rotation or refused credentials say to pair
  again. The certificates the server announces are saved only while the saved connection is still that server, before the link tells the server
  it has the next one. A server that moved is followed through the browser, and the new address is
  saved only while nobody saved another connection meanwhile, with a
  `serverMoved` entry in the activity log.
- **USB:** the app runs koppel-usb's `UsbServer`, `UsbDataPlane`, `UsbClient` and `UsbIpGateway` on
  what `koppel` picks from the settings: sysfs on Linux (`Usb:SysRoot`), a USB/IP server to share
  from (`Usb:Upstream`), usbip-win2's `usbip.exe` on Windows (`Usb:UsbipPath`, else the bundled copy,
  else a standalone install), as 0.1.x did.
- **Tests:** `koppel-web/tests/roles.rs` starts a Rust server and a Rust client in one process, each
  on a temporary data folder with a fake kernel. They pair through the code, a shared device
  attaches and its traffic crosses both relays and Skakel's stream, the link and the device survive a
  certificate handover and a server restart, revoking and rotating end the link with word to pair
  again, and leaving the server role ends it. Off Windows, where the client's discovery socket could
  bring up a firewall prompt, the link also follows a server that moved. Callers at 127.0.0.1 and
  127.0.0.2 do not share a pairing limit through Koppel's port, and the server's list shows the
  second one's address. The contract tests run Skakel's parts against the Rust build too.
