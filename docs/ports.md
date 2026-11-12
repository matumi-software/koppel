# Ports

## Ports and what they serve

One HTTPS port, 5240, with the server's own self-signed certificate, serves both surfaces: the UI, for the
people who run this machine, and the network surface, for other Koppel machines. `Ports:Network` moves
the network surface onto a port of its own. Which port serves what is decided by the local port the
connection arrived on, never the `Host` header. A request for a surface that is not on that port, or
that this machine's current role does not have, gets 404. Plain HTTP on a web port gets a 301 to the
same host, port and path over HTTPS, answered before TLS from the first byte of the connection
(`koppel-web/src/redirect.rs`); it never reaches the app. The role is read from the settings on every request, so a role change takes effect at once.
The table is `LISTED` in `koppel-web/src/surface.rs`; a path not listed counts as the UI's,
so a new route is never exposed to remote clients by accident. Everything Skakel maps is under `/skakel`,
which is on the network surface.

| Port | Surface | Serves | Role |
|---|---|---|---|
| 5240 HTTPS (`Ports:Ui`) | UI | The UI (single page app), `/api/auth/*`, `/api/settings`, `/api/system/*`, `/api/activity`, the live push (`/api/events`), `/healthz` | Any |
| 5240 HTTPS (`Ports:Ui`) | UI | `/api/server/*` | Server or both |
| 5240 HTTPS (`Ports:Ui`) | UI | `/api/client/*` | Client or both |
| 5240 HTTPS (`Ports:Network`) | Network | Skakel under `/skakel`: info, pairing, tokens and hello. Skakel's and Koppel's gRPC calls: the control call, the remote device list and USB traffic. Koppel's own `/api/remote/*` behind a client's token | Server or both |
| 5240 UDP (`Discovery:Port`) | Network | Answers to discovery probes | Server or both |
| 3242 plain | Gateway for a USB/IP driver on this machine | Client or both, and only where `Gateway:Port` is set or on native Windows |

Settings that move them:

| Setting | Default | What |
|---|---|---|
| `Ports:Ui` | 5240 | The UI's port |
| `Ports:Network` | `Ports:Ui` | The network surface's port. Set it to split the two |
| `Discovery:Port` | `Ports:Network` | The UDP port a server answers probes on and a client sends them to. A client only finds servers that use its own discovery port |
| `Ports:Address` | Any address | The address the web ports bind to. Must be an IP address, or start-up fails naming the setting. Other Koppel machines must reach the network surface, so to keep the UI private use `Ui:AllowFrom` |
| `Ui:SetupWindowMinutes` | 10 | How long a machine without a UI password takes its first one, after it starts or after a reset. Whole minutes from 1 to 1440, or start-up fails naming the setting. See ["UI sign-in"](security.md#ui-sign-in) |
| `Ui:AllowFrom` | `lan`; `local` in Development | Who may use the UI, by the caller's address, as Sunshine's `origin_web_ui_allowed`. `local` is this machine only, `lan` adds the local network, `any` is everyone. A refused UI request gets 403; the network surface is not affected. The local network is loopback, 10/8, 172.16/12, 192.168/16, 169.254/16, IPv6 unique local and link-local addresses, and any network this machine has an address on (read again every 30 seconds), so a neighbour with a global IPv6 address counts. Carrier-grade NAT (100.64/10) does not. The Windows installer sets `local`, because the native app's UI is used on that machine only |
| `Gateway:Port` | 3242 on native Windows, off elsewhere | See ["USB client"](usb.md#usb-client). A compose file sets it by environment variable |
| `Gateway:Address` | `127.0.0.1` | Where the gateway listens |
| `RateLimits:TokenPerMinute`, `RateLimits:PairPerMinute` | 60, 120 | Skakel's limits on its token and pairing endpoints, per address. In the Rust build Skakel counts them behind the port and role check, so a call to the wrong port or role does not count |
| `RateLimits:AuthPerMinute` | 30 | See ["Guards on the web surface"](security.md#guards-on-the-web-surface) |

`GET /healthz` is on the UI port, anonymous, answers `ok` in any role and needs no header. The Docker
image's health check is Koppel itself (`koppel --healthcheck`, `koppel/src/healthcheck.rs`), because the port is HTTPS and the image has no curl or openssl. It calls `/healthz` on `Ports__Address` and `Ports__Ui` (any-address and unset mean loopback), then on 127.0.0.1, and accepts any certificate, since it sends nothing secret. Plain Docker only marks the container unhealthy and does not restart it; `restart:` acts on exits. Swarm and Kubernetes act on it, and with plain Docker something such as an autoheal container has to.
