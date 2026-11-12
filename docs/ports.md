# Ports

## Ports and what they serve

One HTTPS port, 5240, with the server's own self-signed certificate, serves both surfaces: the UI, for the
people who run this machine, and the network surface, for other Koppel machines. `KOPPEL_NETWORK_PORT` moves
the network surface onto a port of its own. Each surface is a router (`koppel-web/src/routes.rs`) with its
own guards, and a router answers on the listener it is given, so the port decides the surface by
construction: with `KOPPEL_NETWORK_PORT` set, a UI path on the network port is 404 and a network path on the UI
port is 404. On one port the two routers sit behind a small dispatcher that hands a request to the
network router when its path is `/skakel/...`, `/api/remote/...` or a gRPC call (`is_network_path` in
`koppel-web/src/surface.rs`, the one place that names them) and to the UI router otherwise. A request's
surface comes from its path and the port it came in on, never from a header or the `Host` header: a request
that says it is gRPC (any `application/grpc+...` content type) is the UI's when its path is, and gets the
UI's guards. A gRPC call is told by its path, `/<package>.<Service>/<Method>` with a package under
`skakel.` or Koppel's `koppel.remote.v1.RemoteDevices`. Paths are matched as they are written, in the case
they are written: `/API/settings` is not `/api/v1/settings`. A machine whose role lacks a surface does not
serve it: 404, read from the settings on every request, so a role change takes effect at once. Plain HTTP
on a web port gets a 301 to the same host, port and path over HTTPS, answered before TLS from the first
byte of the connection (`koppel-web/src/redirect.rs`); it never reaches the routers. A new route joins a
router and gets its layers, so it is never exposed to remote clients by accident: the UI's routes are under
`/api/v1` in `koppel-web/src/api/mod.rs`, the network surface's are Skakel's.
Everything Skakel maps is under `/skakel`,
which is on the network surface. Pairing there works only while a person has opened the pairing
window on the server's Clients page (`PUT /api/v1/server/pairing/window` on the UI surface, see
["Finding servers and pairing"](skakel.md#finding-servers-and-pairing)); outside it Skakel answers
`POST /skakel/v3/pair` with 403.

| Port | Surface | Serves | Role |
|---|---|---|---|
| 5240 HTTPS (`KOPPEL_UI_PORT`) | UI | The UI (single page app), `/api/v1/auth/*`, `/api/v1/settings`, `/api/v1/system/*`, `/api/v1/activity`, the live push (`/api/v1/events`), `/healthz`, `/readyz` | Any |
| 5240 HTTPS (`KOPPEL_UI_PORT`) | UI | `/api/v1/server/*` | Server or both |
| 5240 HTTPS (`KOPPEL_UI_PORT`) | UI | `/api/v1/client/*` | Client or both |
| 5240 HTTPS (`KOPPEL_NETWORK_PORT`) | Network | Skakel under `/skakel`: info, pairing, tokens and hello. Skakel's and Koppel's gRPC calls: the control call, the remote device list and USB traffic. Koppel's own `/api/remote/*` behind a client's token | Server or both |
| 5240 UDP (`KOPPEL_DISCOVERY_PORT`) | Network | Answers to discovery probes, over IPv4 and over IPv6 when the web ports serve IPv6 | Server or both |
| 3242 plain | Gateway for a USB/IP driver on this machine | Client or both, and only where `KOPPEL_GATEWAY_PORT` is set or on native Windows |

Settings that move them:

| Setting | Default | What |
|---|---|---|
| `KOPPEL_UI_PORT` | 5240 | The UI's port |
| `KOPPEL_NETWORK_PORT` | `KOPPEL_UI_PORT` | The network surface's port. Set it to split the two |
| `KOPPEL_DISCOVERY_PORT` | `KOPPEL_NETWORK_PORT` | The UDP port a server answers probes on and a client sends them to. A client only finds servers that use its own discovery port. A server answers on this port over IPv4 and, when `KOPPEL_LISTEN_ADDRESS` is unset or an IPv6 address, over IPv6 as well; a client probes both |
| `KOPPEL_LISTEN_ADDRESS` | Any address | The address the web ports bind to. Must be an IP address, or start-up fails naming the setting. Other Koppel machines must reach the network surface, so to keep the UI private use `KOPPEL_UI_ALLOW_FROM` |
| `KOPPEL_UI_SETUP_WINDOW_MINUTES` | 10 | How long a machine without a UI password takes its first one, after it starts or after a reset. Whole minutes from 1 to 1440, or start-up fails naming the setting. On Linux and in Docker, with `KOPPEL_UI_ALLOW_FROM` other than `local`, the first password also needs the setup token printed in the log. See ["UI sign-in"](security.md#ui-sign-in) |
| `KOPPEL_UI_ALLOW_FROM` | `lan`; `local` in Development | Who may use the UI, by the caller's address, as Sunshine's `origin_web_ui_allowed`. `local` is this machine only, `lan` adds the local network, `any` is everyone. A refused UI request gets 403; the network surface is not affected. The local network is loopback, 10/8, 172.16/12, 192.168/16, 169.254/16, IPv6 unique local and link-local addresses, and any network this machine has an address on (read again every 30 seconds), so a neighbour with a global IPv6 address counts. Carrier-grade NAT (100.64/10) does not. The Windows installer sets `local`, because the native app's UI is used on that machine only |
| `KOPPEL_GATEWAY_PORT` | 3242 on native Windows, off elsewhere | See ["USB client"](usb.md#usb-client). A compose file sets it by environment variable |
| `KOPPEL_GATEWAY_ADDRESS` | `127.0.0.1` | Where the gateway listens |
| `KOPPEL_RATE_TOKEN_PER_MINUTE`, `KOPPEL_RATE_PAIR_PER_MINUTE` | 60, 120 | Skakel's limits on its token and pairing endpoints, per address. In the Rust build Skakel counts them behind the network router and its role gate, so a call to the wrong port or role does not count |
| `KOPPEL_RATE_AUTH_PER_MINUTE` | 30 | Per whole address, on setup, login and password change. The sign-in lock itself counts an IPv6 /64 as one caller, and a global budget stops a flood. See ["Guards on the web surface"](security.md#guards-on-the-web-surface) |

`GET /healthz` is on the UI port, anonymous, answers `ok` in any role and needs no header. `GET /readyz` follows the same rules and answers a small JSON object, 200 when Koppel is ready and 503 when it is not: `{"ready":true,"gateway":"off","certificate":"loaded","link":"unconfigured"}`. `gateway` is `off`, `listening` or `faulted` (the port is taken), `certificate` is `loaded` or `missing`, `link` is the client's link (`unconfigured`, `connecting`, `connected`, `error`). It is not ready when the gateway is faulted or no certificate is loaded; a link that is down is reported but does not decide it, since a client whose server is away is working. It names states only, never a message, address or fingerprint, because anyone `Ui:AllowFrom` lets in can read it; the reason is on the devices page and in the log. `/healthz` stays the liveness check for Docker and the systemd watchdog. The Docker
image's health check is Koppel itself (`koppel healthcheck`, `koppel/src/healthcheck.rs`), because the port is HTTPS and the image has no curl or openssl. It calls `/healthz` on `KOPPEL_LISTEN_ADDRESS` and `KOPPEL_UI_PORT` (any-address and unset mean loopback), then on 127.0.0.1, and accepts any certificate, since it sends nothing secret. Plain Docker only marks the container unhealthy and does not restart it; `restart:` acts on exits. Swarm and Kubernetes act on it, and with plain Docker something such as an autoheal container has to.
