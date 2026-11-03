# Koppel architecture

Koppel shares USB devices over the network. A machine runs as a **server** (offers the devices
plugged into it), a **client** (attaches devices from a server so they appear local), or both.
One codebase and one Docker image cover every role; the role is a setting.

## Layers

| Layer | What | Status |
|---|---|---|
| Control plane | ASP.NET Core host, REST for commands, SignalR for pushes, React UI behind a password | Done |
| Auth | The server is an OAuth server (client credentials); client ID and secret are kept encrypted next to the config on both sides. The UI has its own password | Done |
| Data plane | USB/IP protocol (Linux kernel spec), in a WebSocket on the HTTPS port | Linux server and client work. A Windows client works through usbip-win2 (see "Real hardware" below). A Windows server shares through usbipd-win and has been tried only against a fake |

## The confirm rule

The UI never assumes a change worked.

1. The UI sends the change over REST (`PATCH /api/settings`). The server answers `202 Accepted` with no body.
2. The server writes the change to its JSON config file (temp file, flush, move over the target).
3. Only after the write lands does the server push the confirmed state over SignalR (`SettingsChanged` on `/api/hubs/ui`).
4. The UI shows the control as pending from step 1 until the pushed value matches what the user asked for. A rejected request (400) or no confirmation within 10 seconds puts the control back to its confirmed value with the reason.

Hand edits to the config file are picked up by a file watcher and pushed the same way (see "Config
on disk" for what happens to a bad edit). While the SignalR link is down, every control that sends
a change is disabled, and the UI refetches over REST after reconnecting.

How the UI applies it:

- `HubPushes` (under `HubProvider`) applies every hub event to the query cache for the whole app. Pages register no handlers of their own. `nearby-servers.tsx` keeps one local handler that only ends its own "starting" state.
- Pending changes live in `PendingProvider`, above the pages, one store per domain (`pending/domains.ts`: settings, serverDevices, clientDevices, clients, connection, pairingRequests, pairing). Each pending change has an activity line. Approve and Refuse on a pairing request use the pairingRequests store like any other change. Setup is a pending change in the settings domain. Leaving a page orphans nothing.
- The provider watches the query cache and judges a pending change whenever confirmed data lands, from a push or from the refetch after a reconnect. A change that failed clears, and counts as saved, once the confirmed value equals the value that failed.
- A push cancels the queries in flight for that key before it writes to the cache, so a late fetch cannot overwrite it. The activity list is the exception: a push merges one event in by id and the fetch merges with the cache after it lands, so neither loses the other's events.
- A rejection or timeout that arrives after a change settled is ignored, and a late answer to an old send cannot settle a newer one.
- The connection form remounts on a save that was pending when confirmed, so the secret leaves the page state. A late confirm of a save that had failed does not remount it, and a changed stored connection is shown without dropping edits the person has made.
- Setup sends one `PATCH { role, setupComplete: true }`.
- Changing the password completes on the REST response, because there is no pushed value to wait for (commented in `change-password.tsx`).

Code (under `src/Koppel/`): `src/Koppel/Settings/SettingsStore.cs` (write, then raise),
`src/Koppel/Hubs/UiHub.cs` (relay to the hub), `src/Koppel.UI/src/hub/hub-pushes.tsx` (pushes into
the cache), `src/Koppel.UI/src/pending/` (pending, confirm, fail), `src/Koppel.UI/src/settings/use-settings.ts`.
The end-to-end test is `tests/Koppel.Tests/Settings/SettingsConfirmFlowTests.cs`; the UI has its own
tests (`pnpm --filter koppel.ui test`).

## Ports and what they serve

Two listeners, both HTTPS with the server's own self-signed certificate. The decision uses the local port the connection arrived on, never the `Host` header.
A request for a surface the other listener owns, or that this machine's current role does not have,
gets 404. The role is read from the settings on every request, so a role change takes effect at once.
The table is `Surface.Listed` in `src/Koppel/Web/SurfaceGuard.cs`; a path not listed counts as the UI's,
so a new route is never exposed to remote clients by accident.

| Port | Serves | Role |
|---|---|---|
| 5240 HTTPS | The UI (single page app), `/api/auth/*`, `/api/settings`, `/api/system/*`, `/api/activity`, `/api/hubs/ui`, `/healthz` | Any |
| 5240 HTTPS | `/api/server/*` | Server or both |
| 5240 HTTPS | `/api/client/*` | Client or both |
| 5241 HTTPS | `/connect/token`, `/api/remote/*` (USB traffic at `/api/remote/usb`), `/api/hubs/remote`, `/api/pair*` | Server or both |
| 5241 UDP | Answers to discovery probes | Server or both |
| 3242 plain | Gateway for a USB/IP driver on this machine | Client or both, and only where `Gateway:Port` is set or on native Windows |

Settings that move them:

| Setting | Default | What |
|---|---|---|
| `Ports:Http`, `Ports:Https` | 5240, 5241 | The UI port and the port for other Koppel machines. Both are HTTPS; the names are older than that |
| `Ports:HttpAddress` | Any address; loopback in Development | The address the UI port binds to. Must be an IP address, or start-up fails naming the setting. The Windows installer sets `127.0.0.1` for the service, because the native app's UI is used on that machine only. Docker keeps any address, so a headless machine's UI is reachable on the LAN |
| `Gateway:Port` | 3242 on native Windows, off elsewhere | See "USB client". A compose file sets it by environment variable |
| `Gateway:Address` | `127.0.0.1` | Where the gateway listens |
| `RateLimits:TokenPerMinute`, `RateLimits:PairPerMinute`, `RateLimits:AuthPerMinute` | 60, 120, 30 | See "Guards on the web surface" |

`GET /healthz` is on the UI port, anonymous, answers `ok` in any role and needs no header. The Docker
image's health check is Koppel itself (`dotnet Koppel.dll --healthcheck`, `Web/HealthProbe.cs`), because the port is HTTPS and the image has no curl or openssl. It calls `/healthz` on `Ports__HttpAddress` and `Ports__Http` (any-address and unset mean loopback), then on 127.0.0.1, and accepts any certificate, since it sends nothing secret. Plain Docker only marks the container unhealthy and does not restart it; `restart:` acts on exits. Swarm and Kubernetes act on it, and with plain Docker something such as an autoheal container has to.

## Auth

- **Certificate:** each server creates a self-signed certificate on first start. Its SHA-256 fingerprint shows in the server's Clients section. Clients pin it: they talk only to a server presenting that exact certificate.
- **Clients:** the server admin adds a client by name and gets a client ID and secret. The secret is kept inside the encrypted `secrets.json` on purpose, so the admin can copy it again later from the client's row. The server verifies a presented secret against a SHA-256 hash. A secret is 32 random bytes, so a fast hash is enough there. A client made before this change has only a legacy PBKDF2 hash: it still verifies, is upgraded to SHA-256 on its next successful token request, and has no copyable secret until it is rotated.
- **Revoking:** revoking a client, or rotating its secret, blocks it at once. Each registered client has a generation number, carried in its tokens as the claim `koppel_generation`. The USB handshake, the remote hub and `/api/remote` accept a token only for a client that is still registered at that generation. `LiveSessions` also ends what the client is doing right now: its USB relays and its hub connections are aborted when it is revoked or rotated. A new token request with the old secret fails. The generation in a token is the one the secret was verified against, carried from the check to the issue, so a rotation in between never gives a token for the newer generation.
- **Tokens:** OAuth 2.0 client credentials, issued by OpenIddict in degraded mode (no database; clients come from our encrypted file). Access tokens are signed with an RSA key kept in the encrypted secrets file, last 30 minutes, and are only accepted over HTTPS.

### UI sign-in

The UI on port 5240 (HTTPS) needs a password. There is one admin password and no user name.

- **First run:** while no password is set, `POST /api/auth/setup` needs a one-time 8 character setup code, even from the same machine. The code is made of letters and digits without I, O, L, 0 and 1. Koppel logs it at Warning and writes it to `setup-code.txt` in the config directory. Setting the password removes the file.
  - Docker: `docker logs <container>`, or `/koppel/config/setup-code.txt` in the container.
  - Windows: `%ProgramData%\Koppel\config\setup-code.txt` (administrators and SYSTEM can read it).
  - Setup is step 1 of the first-run wizard in the UI.
- **Password:** at least 10 characters, at most 128, and not "koppel" or "password" in any case. It is hashed with PBKDF2-SHA256, 600,000 iterations and a 16 byte salt, and kept with a security stamp in the encrypted `ui-auth.json`.
- **Cookie:** `koppel.ui`, HttpOnly, SameSite=Strict, Secure (the UI is HTTPS only), a session cookie that lasts 12 hours and slides while it is used, and never more than 7 days after the sign-in (the password change does not restart that). The security stamp, the session id and the 7 day cap are checked on every request. An API call that is not signed in gets 401, never a redirect.
- **Backoff:** login, password change and setup lock a caller for one minute after 5 wrong tries, and double the lock on each further failure, up to one hour. A caller is one IPv4 address or one IPv6 /64, as in pairing. A try is counted before the password is hashed and given back when it is right, so parallel guesses cannot all get in ahead of the lock. The lock applies before the password is looked at, so a right guess during it is refused too. Setup has no limit across callers: the code is about 40 bits, so a global lock would let one host on the network keep the admin out. The table of callers holds at most 2048 entries.
- **Hash slots:** at most 2 password hashes run at once for the whole server (setup, login and password change). A caller who arrives while both are busy gets 429 with `Retry-After: 1` at once, and the try is not counted against it.
- **Failures in the log:** every failure goes to the log. The activity log gets one `signInFailed` entry each time a lock engages, so a burst of guesses cannot push the 500 other entries out.
- **Ending sessions:** a password change, a reset or signing out invalidates the cookies and closes the UI hub connections of the sessions that ended. Signing out also revokes the session id on the server, kept until the session could no longer have been valid, so a copy of the cookie is refused too. The session that changed the password stays signed in. The UI hub is mapped with `CloseOnAuthenticationExpiration`, so a connection also ends when its cookie runs out. In the browser the sign-in gate (`AuthGate`) shows the login screen on any 401, and re-asks the server whether the session is still good when the hub closes and on every reconnect retry, so a session that ended during an outage does not retry forever. Signing out cancels any auth check in flight and ignores rechecks until it finishes, and a failed sign-out says so.
- **Where the rule lives:** `Surface.RequiresSignIn` and `Surface.Anonymous` in `SurfaceGuard.cs`. Every `/api/*` route and `/api/hubs/ui` on the UI port needs a session by default. Anonymous: `/api/auth/state`, `/api/auth/setup`, `/api/auth/login`, `/healthz` and the SPA files. The remote surfaces on 5241 never look at the cookie; they keep bearer tokens.
- **Forgot the password:** run the reset command. It clears the password, changes the stamp (so every session ends) and exits. A running service sees the change through its file watcher and issues a new setup code at once; otherwise the next start does. Then read the code and set a new password in the UI. An unreadable `ui-auth.json` is replaced by the reset.

  ```sh
  docker compose exec koppel dotnet Koppel.dll --reset-password   # or: docker compose run --rm koppel --reset-password, if it is not running
  ```

  ```powershell
  # elevated prompt
  & "C:\Program Files\Matumi\Koppel\Koppel.exe" --reset-password
  ```

### Guards on the web surface

- **Origin and header (UI port):** an `Origin` that does not match the request's own scheme, host and port gets 403 on any `/api/*` call, the hub's WebSocket upgrade included. Behind a TLS proxy the scheme comes from `X-Forwarded-Proto`. A change (anything but GET or HEAD) also needs `X-Koppel-UI: 1`, otherwise 403. A page on another site cannot add that header without a CORS preflight, which this server never grants. The UI adds it in `api/http.ts` (`uiFetch`, used by every mutating fetch). The hub is exempt from the header only, because an upgrade cannot carry one.
- **Host header:** `AllowedHosts` stays `*` and is configurable. A DNS rebinding page is stopped by HTTPS: the browser checks Koppel's certificate against the rebinding page's own host name and the request fails, and a page that tries plain HTTP gets no answer from a TLS port. The SameSite=Strict cookie and the Origin and header checks stand behind that, not the host name.
- **Security headers** on every response: `X-Frame-Options: DENY`, `Content-Security-Policy: frame-ancestors 'none'` (only that, so it cannot break the app), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`.
- **Rate limits:** a fixed one-minute window per remote IP, answering 429. `/connect/token` allows 60 (`RateLimits:TokenPerMinute`), `/api/pair*` allows 120 (`RateLimits:PairPerMinute`; a waiting client polls 30 times a minute), and `/api/auth/setup`, `/login` and `/password` allow 30 (`RateLimits:AuthPerMinute`), counted per IPv4 address or IPv6 /64. The limiter sits after the port and role check, so a request for a surface that is not there does not use up an allowance.
- **Remote hub:** mapped with `CloseOnAuthenticationExpiration`, so a connection ends when its token runs out. The client makes the link again 30 seconds before that, with a fresh token, and shows nothing of it: no "connecting" and no entry in the activity log. A close within 10 seconds of the token's end counts as the same expiry.
- **Connection test:** the client's Test button gets one of a fixed set of outcomes with fixed wording, never raw exception text: `ok`, `incomplete`, `unreachable`, `certificateMismatch`, `rejectedCredentials`, `clientUnknown`. The detail goes to the log.
- **Dev proxy:** the Vite dev server proxies `/api` to `KOPPEL_UI_URL`, default `https://127.0.0.1:5240`, the UI port, without verifying Koppel's self-signed certificate. It sets `X-Forwarded-Proto` so the Origin check passes.

### Threat model

What the design holds against:

- A stranger on the network reaching the UI: the UI needs a password, and the first password needs the setup code that only someone with access to the machine's log or config directory sees. Guessing is slowed by the backoff.
- A web page in the admin's browser calling the UI: SameSite=Strict, the Origin check and the `X-Koppel-UI` header.
- A stranger reaching the remote surface: the HTTPS port serves only the token endpoint, pairing and the token-guarded remote API, and each of those has a rate limit. Nothing from the UI is served there.
- A revoked client: its sessions end and it cannot get a new token.

What it does not cover:

- The browser cannot tell Koppel's self-signed certificate from an attacker's. It warns the first time, and whoever accepts the warning on a network they do not trust can be shown an impostor that collects the password. The certificate's SHA-256 fingerprint is the one under Clients, so it can be checked in the browser's certificate details. On Windows the installer binds the UI to the loopback address. The Docker Desktop compose files for Windows publish it on `127.0.0.1`.
- The gateway on 3242 is plain USB/IP with no login (see "USB client"). Whoever reaches it uses the client's access to the server.
- Anyone who can read the keys directory can decrypt the secret files, unless `Paths:KeyCertificate` protects the key ring (see "Config on disk").
- The Linux container is privileged and runs as root, and the Windows service runs as LocalSystem. A flaw in Koppel is a flaw with that reach.

## Finding servers and pairing

- **Discovery:** every ten seconds a client broadcasts a probe over UDP to its HTTPS port number (5241 unless changed), on every IPv4 network it is on. A server listens on UDP at its own HTTPS port number and answers the client directly with its name, HTTPS port, certificate fingerprint and version (`KoppelService`). The client reaches the server at the address the answer came from. A client lists what it hears under Configuration, Servers on your network. `Discovery:Enabled=false` turns both sides off.
- **Why not multicast DNS:** it worked, but it needs UDP 5353 as well, and Koppel's aim is one port number between machines. Probes are padded to 512 bytes and an answer is never larger than the probe, so a server cannot be used to multiply traffic at someone else. Broadcasts stay on the local network, so servers elsewhere are still entered by address. IPv6-only networks are not covered yet.
- **Discovery limits:** the client keeps at most 64 entries, at most 8 from one source address; when a limit is hit, the stalest entry of the address holding the most makes room, so one address cannot push out the others. Entries are keyed by fingerprint and the address the answer came from, so a server that copies a name shows beside the real one and the duplicate-name warning fires. The list is pushed only when it changed, and a slow listener gets the latest list, not every one in between. An entry needs a fingerprint of exactly 64 hex digits (shown in colon form), a name of at most 63 characters and a version of at most 32; anything else is dropped. An entry that missed three probes in a row (35 seconds) expires. Every start, stop and probe runs under one lock, and a probe checks under it that browsing is still wanted, so a role change cannot be undone by a tick. The timer catches every exception and logs it. The list shows a short fingerprint beside each name and warns about a duplicate name or one that imitates another.
- **Pairing:** instead of copying a client ID and secret, the client shows a six-digit code and a person types it on the server's Clients page, under Waiting to pair, as with Moonlight and Sunshine. On a match the server registers the client and hands it its credentials over HTTPS pinned to the announced fingerprint; they go straight into `connection.json`, and `ConnectionChanged` confirms it as for any saved connection.
- **Replacing a connection:** if the client already has a connection to a different server, an approved pairing does not overwrite it. The client goes to a `confirm` state that shows the new server's name and fingerprint, and the person chooses Replace or Discard (`POST /api/client/pairing/confirm` to replace, `DELETE /api/client/pairing` to discard). The held credentials stay in memory until the person acts, or pairing is started again or cancelled. The `paired` state clears once the saved connection no longer matches it.
- **Why the code can be trusted:** it is numeric comparison with a commitment, as in Bluetooth pairing. The client sends a hash of a random nonce, the server answers with its own nonce, then the client reveals its nonce. Both ends derive the code from the pinned certificate and both nonces (`PairingCode`). A man in the middle has its own certificate and must fix each nonce before it sees the other side's, so it cannot steer the two codes to match.
- **Poll token:** opening a request returns a random 32 byte poll token, which only the opener sees. Polling and withdrawing need it in the `X-Pair-Token` header, compared in constant time. It is in no view and no push, so another person on the network who sees the request in the list cannot read the credentials.
- **Limits, per server:** 60 opens per 5 minutes in total and 5 per address (an IPv4 address or an IPv6 /64), never refunded, so one address cannot use up what the others need while many addresses still cannot grind. When the total is spent, `PairingRequestsChanged` and `GET /api/server/pairing` carry `budgetExhausted: true` and the Clients page says the server is turning requests away; at most 10 revealed waiting requests (a request counts only once the client has revealed its nonce); at most 3 open requests per address (an IPv4 address or an IPv6 /64). A request that has not revealed its nonce within 30 seconds is dropped. Requests expire after five minutes. Three wrong codes refuse the request, which then shows as `denied` in the server's list for 15 seconds so the person sees why. A client may withdraw its request. Approve, deny, reveal, poll and withdraw on one request run one at a time. The chance of guessing one code is about one in a million, so what bounds an attacker is how many requests the server takes, which these limits fix.
- **Endpoints:** `/api/pair` (HTTPS only, no token, refused on a client-only machine) for the exchange; `/api/server/pairing` for the server's UI; `/api/client/discovery` and `/api/client/pairing` for the client's UI. Pushes: `ServersDiscovered`, `PairingStatusChanged`, `PairingRequestsChanged` (an object: `requests` and `budgetExhausted`). The client's pairing status carries the server's fingerprint, for the confirm step.
- **Saved connection:** `PUT /api/client/connection` keeps the stored secret only when the server address and fingerprint are unchanged. Otherwise the request must carry a new secret, or it is refused with 400.
- **Docker:** broadcasts do not cross Docker's bridge network, so the Linux server compose file uses the host network. On Windows, Docker Desktop's host networking does not reach the LAN: use the native install there. A Windows client needs no firewall rule: Windows lets the direct answers to its own broadcast in.

## USB data plane

### USB server

- **Host:** `IUsbHost` says what a host can do instead of throwing or pretending: `CanShare`, `CanUnshare`, `CanReset`, and `SharingAdvice` (the command to run on the machine that does the sharing, such as `usbipd bind`). Its methods are async and take a cancellation token. `ExportAsync(device, stream, exported, ct)` owns the relay and returns when the session ends; `exported` runs before any USB traffic, which is when Koppel answers the client's import request.
- **Linux host:** `UsbServer` polls sysfs every two seconds and applies the share choices kept in `devices.json`, which holds the bus IDs and the VID:PID each had when shared. It re-shares a device when it is plugged back in. `SysfsUsbHost` binds a device to `usbip-host`, and takes the bus ID off `match_busid` again when a bind fails. The poll survives any error from the host: it logs it and asks less often until the host answers again.
- **Pushing the list:** `UsbServer` takes each snapshot of the device list, and its sequence number, under a lock and delivers it to `Changed` handlers outside it, in order, through one pump that drops snapshots a newer one overtook. A handler that never returns holds up only the pump, never an import, a share or the poll. An import or release asks for a refresh and does not wait for it.
- **Exportable rule:** a device can be exported only when it is bound and is the VID:PID that was shared on that bus port. A bus ID is a port, so a different device plugged into the same port is unbound, shows a fault and is not shared without asking. A share whose apply fails is not saved. Bus IDs are validated (`UsbBusId`) before anything is saved; the endpoint answers 400.
- **Start-up reconciliation:** at start, Koppel shares what was saved (when `ShareOnStartup` is on and the role serves) and unbinds every other device bound to `usbip-host`, such as a share a crash left behind. A sysfs error here is logged and does not stop start-up; the poll applies the saved shares on its next pass.
- **Role change:** the surface guard serves the data plane only while the role is server or both. Leaving the server role ends the sessions and aborts the remote hub connections (`LiveSessions`), and unshares the devices in the background, so the settings change is confirmed to the UI without waiting for the unshare.
- **Session:** a client opens a WebSocket at `/api/remote/usb` on the HTTPS port, pinned to the server's certificate and with its bearer token. The remote group checks the token and that the client is registered at its current generation (a missing or invalid token gets 401), and `UsbDataPlane` then speaks plain USB/IP inside the WebSocket: a device list, or an import followed by the device's traffic. The client has 10 seconds to ask for one. Before it is authenticated a caller gets only what Kestrel gives any HTTPS request: its TLS handshake and request header timeouts and the token check. A session ends when its client is revoked or rotated, when the role leaves the server side, when either end misses pings for 30 seconds, or when Koppel stops.
- **Import:** the status check, the reset and the start of the export run under a lock per bus ID, so two imports of one device cannot race the reset. An upstream host (usbipd-win) claims the bus ID before it connects to the upstream server and releases it after the connection is closed.
- **Relay:** on import the server writes one end of a loopback socket pair to the device's `usbip_sockfd`, so the kernel serves USB from there, and relays the other end through the WebSocket. The loopback pair accepts only the kernel end's own connection. Both ends ping every 15 seconds and give up after 30 without an answer, so a client that vanished frees its device.
- **Revocation:** see "Auth". A USB session carries the generation of its token and ends when the client is revoked or rotated.
- **Live list:** clients follow the server's shared devices over the `/api/hubs/remote` SignalR hub (HTTPS, bearer token, pinned certificate). The remote view shows the shared devices with their bus ID, VID:PID, name, manufacturer, speed, state and interfaces. It blanks who holds a device, since when, the serial number, the driver and any fault text: another client learns only that the device is taken.
- **Docker:** USB needs a privileged container running as root, and the host must have the modules loaded (`modprobe usbip-host` on servers, `modprobe vhci-hcd` on clients). Attached devices appear on the client's host kernel.
- **Windows:** the container uses a USB/IP driver on the Windows host over plain USB/IP inside the machine. A server shares what usbipd-win exports (`Usb:Upstream`, `UsbIpUpstreamHost`). The upstream host refreshes in the background with one deadline, so the UI reads a cached snapshot, and caps the number of devices and the bytes it accepts. `Usb:Upstream` is `host`, `host:port`, `[v6]:port` or a bare IPv6 address. A native Windows server still needs `Usb:Upstream` pointed at usbipd-win. See `docs/windows.md`.

### USB client

- **Attach:** the client imports over the WebSocket, writes its loopback end to `vhci_hcd`'s `attach` (Linux) or hands the session to usbip-win2's driver (Windows), and relays. One deadline covers the token, connecting, TLS, the WebSocket, the import reply and the device record: 20 seconds, `Usb:AttachTimeoutSeconds`. A stall becomes a readable error on the device instead of a hang. The import reply must match the bus ID and VID:PID of the device the person picked. Bus IDs from the server are validated on receipt (`UsbBusId`) and invalid ones are dropped.
- **One link:** a saved connection has one token client and one pinned HTTP client, shared by the hub, the attaches and the gateway. The USB WebSocket pings every 15 seconds and closes after 30 without an answer.
- **Fail visibly:** every background loop (link, attach, gateway) catches everything, logs it, shows the state and backs off. Attach, gateway and link failures are worded by one helper (`RemoteFailure.Describe`), for example a certificate that no longer matches the pinned fingerprint.
- **Attachments:** they are keyed by the server's fingerprint plus the bus ID, so two servers with the same bus ID cannot be mixed up. Asking to attach a device that is attached or on its way is a no-op.
- **Connection or role change:** when the saved connection changes, or the role leaves client and both, Koppel detaches and stops everything first. The devices stay remembered in `attached.json`, so they attach again when the new link is up. The release runs in the background (the driver's own command can take 30 seconds), so the settings update, and the UI's confirmation, return at once; an attach that starts meanwhile waits for it. Detaching by hand releases the device first and forgets it after: if `attached.json` cannot be saved, the device is detached and shows a fault. A detach during an attach that is still in flight is noticed when the driver returns, and the driver's attachment is released.
- **Gateway:** `UsbIpGateway` runs only while the role is client or both. It is a plain USB/IP server for a USB/IP driver on this machine (see "Windows" below). It serves at most 64 sessions at once (`Gateway:MaxSessions`) and closes the rest at once. If the port cannot be bound, it tries again with a wait that grows from 5 to 60 seconds, and the reason shows as `gatewayFault` in the client's device list (the API carries it; the UI does not display it yet).
- **Native Windows:** the gateway accepts only connections owned by the kernel (PID 4, which is how usbip-win2's driver connects), looked up in the TCP table. Another process that reaches the loopback port is turned away. This applies on native Windows with an IPv4 loopback address. An IPv6 loopback address is refused outright with a warning. Where a driver is present, the gateway also serves only imports Koppel asked for: before it runs `usbip attach` it books one import for that bus ID, and the gateway accepts exactly one import for the booking. Another local user's `usbip attach`, a device list request, and the driver's own attempts to attach again after a drop are refused; Koppel attaches a remembered device again itself. Elsewhere, such as the Linux container of Docker Desktop, the connection does not come from this machine's process table, so no check applies and the compose file's port publishing is what keeps it local.
- **Stopping:** `StopAsync` detaches every attachment made through the Windows driver (`usbip attach --stop`, then `detach -p`) and stops every relay, so a Linux shutdown does not wait on them. Devices stay remembered. A `--stop` that fails because there was nothing to stop does not fail the attach or the detach.
- **Windows driver:** `UsbipWin2Driver` runs usbip-win2's `usbip.exe`. The bus ID is checked before it is passed, as `--bus-id=<id>`.

### Tested

- **Running the tests:** `dotnet test` runs everything. The discovery round trip (`DiscoveryTests.A_browser_finds_a_server_and_drops_it_when_it_stops_answering`) runs over loopback, so it needs no network.
- **Fakes:** the protocol encoding, the sysfs adapters against a fake `/sys`, and a full server-to-client run on real Kestrel with fake kernels on both ends (token, pinning, hub, WebSocket, import, byte round trip). All the server and client changes listed above (deadlines, revocation, the exportable rule, reconciliation, the gateway check, detach on stop) are covered this way.
- **Not on real hardware yet:** the changes made after the hardware runs below have not been re-run on a real kernel or a real usbip-win2: real `usbip-host` bind and unbind, `usbip_sockfd`, `vhci_hcd`, keepalive against a vanished peer, usbipd-win, and the `--bus-id=` form of the `usbip.exe` command.
- **Real kernel (2026-11-02, WSL2 6.6, software USB device):** sharing binds the device to `usbip-host`, both kernels take Koppel's sockets, and the device enumerates through both relays and TLS up to `SET_CONFIGURATION`. That step deadlocks because the test device and its attached copy sit on the same virtual controller, the only one the WSL kernel has; the same happens with no Koppel in the path. Configuration and bulk data still need a client on a second machine. The rig is in `D:\testing\koppel-usb`.
- **Real hardware (2026-11-02):** a Debian thin client runs the server in Docker and shares a Logitech USB receiver. A Windows 11 client attaches it through the installed service and usbip-win2, and through Docker Desktop's gateway, and it works as keyboard and mouse there. That covers configuration and interrupt transfers on a second machine; bulk transfers (storage, printers) are still untried.

### Windows

The container uses a USB/IP driver on the Windows host over plain USB/IP inside the machine. A server
shares what usbipd-win exports; a client runs a gateway that usbip-win2 attaches from
(`Gateway:Port`, `UsbIpGateway`). See `docs/windows.md`. Installed natively, a Windows client
attaches through usbip-win2's driver, which the installer bundles; on native Windows the gateway is
on by default (port 3242, loopback). A native Windows server still needs `Usb:Upstream` pointed at
usbipd-win.

## Activity

The server keeps an activity log in memory (Koppel.Core.Activity, shaped like Timebase.Core.Activity):
sharing, plugging and unplugging, attaches and releases, client changes, settings changes and failed
sign-ins, each with an outcome and, when it failed, the reason. The UI loads it from `GET /api/activity`
and follows `ActivityRecorded` over the hub, so it is the same after a reload and on every device. A subscriber that fails is logged and does not fail the code that recorded the event. It
starts empty when the process restarts. A change the UI sent shows as Waiting until the server logs
it. A settings change that changes nothing records nothing.

## Running on another machine

`src/Koppel/deploy` runs Koppel on a Debian or Ubuntu machine:

1. Copy `prepare-host.sh` to the machine and run it once with `sudo bash prepare-host.sh`. It installs Docker from Docker's signed apt repository, loads `usbip-host` and `vhci-hcd` and keeps them loaded at boot. If `modprobe` fails (Ubuntu cloud and minimal kernels), it prints the hint to install `linux-modules-extra-<kernel>`.
2. Copy `deploy/docker-compose.yml` there and run `docker compose up -d`. It pulls `matumisoftware/koppel` from Docker Hub.

The compose files pin the image tag (`matumisoftware/koppel:0.1.0`). To upgrade, edit the tag to the
new version, then `docker compose pull && docker compose up -d`. Config and keys stay in the `koppel`
volume.

To run your own build instead, with SSH access to the machine: `bash src/Koppel/deploy/deploy.sh
user@host`. It reads the image name from the deploy compose, builds exactly that image here, copies it
over SSH, and starts it with `deploy/docker-compose.yml`. The compose file's `pull_policy: missing`
means an image already on the machine, such as one `deploy.sh` copied, is never replaced by a pull
from Docker Hub.

The image is built for this machine's architecture; the script stops if the target's differs.

`Paths__KeyCertificate`: by default the key ring sits as plain files in the `koppel` volume.
`deploy/docker-compose.yml` has the commented lines to keep a PFX outside the volume and mount it as
a secret instead, with the `openssl` commands that make one.

## Config on disk

The config directory holds:

- `settings.json`: plain, may be edited by hand.
- `secrets.json`: signing key, HTTPS certificate, registered clients. Encrypted. Created on every machine at first start, whatever the role, because the certificate and the signing key are made then.
- `connection.json` (client role): server address, pinned fingerprint, client ID and secret. Encrypted.
- `ui-auth.json`: the UI password hash and security stamp. Encrypted. `setup-code.txt` sits beside it while no password is set.
- `devices.json` (server role): the bus IDs the user chose to share, with their VID:PID. Plain.
- `attached.json` (client role): the devices this client attached, with the server they belong to. The client attaches them again whenever the server offers them and nobody is using them: after a restart, an upgrade, a dropped link, or a replug on the server. Detaching forgets the device. Plain.

How a file is read and written:

- **Start-up never replaces a file.** Only a missing file is seeded with defaults. A file that exists but cannot be used (unreadable after a few retries, cannot be decrypted, is not valid JSON, or fails validation) stops start-up with an error that names the file, the cause and the likely fix, and the file is left exactly as it was.
- **Two plain files start with defaults instead.** `devices.json` and `attached.json` are not critical and their defaults are safe (nothing shared, nothing remembered), so a corrupt one does not stop Koppel. It starts empty in memory, the file is left untouched, and `GET /api/system/checks` shows a `devices` or `attached` entry ("devices.json was not used.") with the reason. The first change made while the file is bad (sharing a device, attaching one) moves it aside as `<name>.unreadable-<UTC timestamp>`, logs a warning, and writes a fresh file; fixing the file by hand instead clears the entry. `settings.json` and the encrypted files still refuse to start: defaults would put a machine back in the Server role, or lose keys and credentials.
- **Hand edits are validated** the same way a REST change is, including the port rules. While Koppel runs, a bad edit is ignored: the last good value stays, a warning is logged, and `GET /api/system/checks` shows a `settings` entry, "settings.json was not applied", with the reason. A later valid edit clears it. A REST change made over a bad hand edit replaces the bad file with the last good value plus the change.
- **A valid hand edit** is loaded before the next change, so a change from the UI never overwrites it.
- **Pushes are ordered.** `Changed` reaches subscribers in write order, one change at a time, even when several writes are in flight. A handler may update the same file: the nested change is written and applied at once, its delivery is queued behind the running one and anything already waiting, so every subscriber sees the same order, and the nested call returns without waiting for that delivery (it would wait for itself).
- **Writes** go to a random temp name, are flushed, and replace the file; a failed write removes its temp file, the replace is retried on a sharing error, and stale temp files are swept at start.
- **Enums are names only:** a number where a name is expected (`"role": 1`) is refused.

The encrypted files use ASP.NET Core Data Protection with a key ring in a separate keys directory
(`Paths:Keys`). On Windows the key ring is protected with machine DPAPI. On Linux it is plain on disk
unless `Paths:KeyCertificate` points at a certificate kept outside the data volume (for example a
Docker secret); Koppel logs a warning when it is not.

Data folder hardening. Before anything is written, Koppel restricts the config and keys directories:

- **Windows:** the owner becomes Administrators and the ACL is protected (no inherited access) with full control for SYSTEM and Administrators only. A directory that is a junction or a symbolic link is refused. A process that is not elevated, such as a developer running from a console, cannot take ownership, so it also keeps its own access.
- **Linux:** `chmod 700`. A bind-mounted config directory that the process cannot chmod makes start-up fail.

Where the config directory is:

- Development: `src/Koppel/.koppel/config` (gitignored), from `appsettings.Development.json`.
- Docker: `/koppel/config` and `/koppel/keys` inside the `koppel` volume.
- Otherwise: `%ProgramData%\Koppel\config` on Windows, `/var/lib/koppel/config` elsewhere, unless `Paths:Config` is set.

### If the key ring is lost

The encrypted files can be read only with the key ring that wrote them. If the keys directory is
missing, replaced or restored from another machine, Koppel refuses to start and says which file it
cannot use ("Koppel cannot use ...secrets.json: ... The file was not changed ... restore the keys
directory"). It does not make new secrets over the old ones.

- **What you see:** in Docker the container exits and restarts, over and over (`restart: always`), with that message in `docker logs`, until the keys are back. On Windows the service fails to start and the recovery rule retries it; run `Koppel.exe` from an elevated prompt with the service stopped to read the message.
- **What to do:** restore the keys directory from a backup (the keys directory and the config directory belong together). If the keys are really gone, the encrypted files cannot be recovered: move the unreadable ones aside (`secrets.json`, `connection.json`, `ui-auth.json`) and start again. A new `secrets.json` means a new signing key and a new certificate, so every client must be paired again. A lost `connection.json` means entering or pairing the connection again. A lost `ui-auth.json` means setting the password again with a new setup code.

## Installer and Windows service

`src/Koppel/installer` builds `Koppel-Setup.exe`: an MSI inside a bootstrapper that installs the
Visual C++ runtime first. `build.ps1` takes the version from `src/Directory.Build.props`, checks the
vendored usbip-win2 files against `usbip-win2/<version>/SHA256SUMS` and their Authenticode signers
(and fails on any mismatch), restores the packages and npm dependencies from the lock files, and
appends the Windows-only notices.

- **Service:** Koppel runs as a Windows service under LocalSystem, set to start at boot. It is configured to restart after the first, second and third failure, after 5 seconds, and the count resets after a day without one. `sc failureflag \Koppel 1` (the `SetServiceFailureFlag` action) makes a stop with a non-zero exit code count as a failure too, and the host exits that way when a background service dies (`Program.cs` sets `ServiceBase.ExitCode` and `Environment.ExitCode`). The installer sets the service's environment to `Ports__HttpAddress=127.0.0.1`, so the UI answers on this machine only.
- **Firewall:** none. A client only receives answers to its own discovery broadcast, which Windows lets in, and nothing on a client needs to be reached from the network. No rule opens the UI port.
- **Data folder:** `data-folder.ps1` runs as SYSTEM after the files are in. It creates `%ProgramData%\Koppel` with owner Administrators and a protected ACL (SYSTEM and Administrators). An existing folder that is a file or a link, or owned by an account that is not an administrator, fails the install with a message that says what to do (rename or delete the folder, copying keys out first if you need them). An existing folder owned by an administrator (SYSTEM, Administrators, TrustedInstaller, or an account in the local Administrators group) is taken over: its owner and ACL are reset, recursively, any link inside it is deleted (the link only, never what it points to), and a file or folder inside that no administrator owns is renamed aside (`not-admin-owned-<time>`) instead of being taken over. At start the host (`\KoppelPaths.Prepare`) protects `%ProgramData%\Koppel` itself the same way, when the config and keys folders are in it. A machine where the folder is owned by an ordinary user needs it renamed or deleted once.
- **Driver:** `driver.ps1` installs usbip-win2's driver after the files and before the service starts. Its output goes to the MSI log (`msiexec /l*v`), not to a file on disk. pnputil may say a restart finishes the driver install: the installer then asks for a restart. It records the packages and the device node it installed in `%ProgramData%\Koppel\driver-state.json`, and uninstall removes only those, and only the ones Windows still lists with an original name of `usbip2_*.inf` (it reads the driver store through `pnputil /enum-drivers /format xml`, then `Get-WindowsDriver`, so a non-English Windows works). The record is used only if an administrator owns it and nobody else can write to it. A restart that pnputil asks for on uninstall is reported to the MSI too.
- **A standalone usbip-win2** (under Program Files\USBip or in the Apps list) is detected first (whatever the record says), logged, used, and left alone: Koppel never changes or removes its driver or device node. A machine with drivers but no record and no standalone usbip-win2 is treated as an earlier Koppel install and adopted. An uninstall with no record leaves the driver in place and says how to remove it by hand.
- **Failure and upgrade:** a fresh install that fails after the driver went in removes the driver again. An upgrade keeps the driver and updates it in place; the driver removal is skipped when an upgrade replaces the product. A same-version MSI replaces the installed one, which keeps a rebuilt dev installer from installing next to the last. A driver that will not come out never blocks an uninstall: the script prints a `WARNING:` with the manual `pnputil` command, and the MSI log has the output. An upgrade from an installer older than this one still runs that old installer's uninstall once, and it removes the driver by name.
- **Notices:** `THIRD-PARTY-NOTICES.txt` is generated by `installer/new-notices.ps1` from the package and npm metadata and committed at `src/Koppel/src/Koppel/`. It ships in the MSI and in the Docker image.

## Building and releasing

Every project restores from a committed `packages.lock.json`, and every csproj restores for `win-x64`,
`linux-x64` and `linux-arm64`, so one lock file covers every publish target. The builds restore in
locked mode **without** `-r`, then publish with `--no-restore -r <rid>`; `dotnet restore -r` fails
against the lock files. A locked build fails with NU1004 when a package changed and the lock file did
not: run `dotnet restore` on `Koppel.slnx` and `Koppel.Core.slnx` and commit the lock files.
The Dockerfile pins the Node, SDK and runtime images to exact versions, fails on an architecture it
does not know, and runs the health check.

To release:

1. Set `<Version>` in `src/Directory.Build.props`.
2. Set the image tag in the three deploy composes (`deploy/docker-compose.yml`, `docker-compose.windows-client.yml`, `docker-compose.windows-server.yml`) and the `KOPPEL_TAG` default in `src/Koppel/docker-compose.yml`.
3. Regenerate the lock files if any package changed.
4. For a new usbip-win2 version, run `installer/export-usbip-win2.ps1` (it rewrites `SHA256SUMS`; `-SumsOnly` regenerates only that), then add the new version to `$usbipWin2Pins` in `installer/build.ps1` (signer thumbprints and the hashes of the two unsigned DLLs, taken from a copy you checked by hand: `SHA256SUMS` alone could be regenerated to match a swapped file) and check which CLI11 tag its licence comes from. Run `installer/new-notices.ps1` when packages changed.
5. Add the version to `CHANGELOG.md`.

## Known limits

- The Linux container is privileged and runs as root. Narrowing it needs a test on a real kernel. The Windows composes also run as root, because the volume belongs to it.
- The installer is not code signed, and there is no CI pipeline or image signing.
- The elevated installer paths have not been run on a VM: taking over an existing data folder (and renaming aside what is not admin-owned), the exit-code recovery (`sc failureflag`, the host exiting non-zero), the driver state and standalone detection, the rollback, the restart request, and whether the service manager reads the `Environment` value the installer writes. Run an install, a major upgrade and an uninstall on a test machine before relying on them.
- The new USB behaviour is covered by fakes, not yet re-run on real hardware (see "Tested").
- `Common.Logging` and `Polly.Extensions.Http` are deprecated packages that arrive through the mDNS library and OpenIddict. Koppel code does not use them directly.
- The UI does not show `gatewayFault` or the live `listen` status yet.

## USB layer decisions (from the feasibility study, 2026-11-02)

- **Protocol: USB/IP**, as documented in the Linux kernel. Standard clients (`usbip attach`, usbip-win2, Docker Desktop) work against it, and we do not write our own virtual USB driver. Koppel's value sits around it: auth, discovery, UI.
- **USB traffic does not go over SignalR or REST.** It uses its own TCP connection; on Linux the kernel takes over the socket. SignalR and REST stay the control channel.
- **USB/IP has no auth or encryption.** Koppel carries it in a WebSocket over TLS with a small relay on each side and only allows an import after an OAuth check. The raw USB/IP port is never exposed to the network.
- **Platforms:**
  - Linux server: the kernel's `usbip-host`, driven through sysfs. Docker image, privileged.
  - Linux client: the kernel's `vhci-hcd`, driven through sysfs. Attached devices appear on the host kernel, not only in the container.
  - Windows server: start by driving an installed usbipd-win through its CLI (not bundled, so its GPL licence does not reach us). Our own libusb/UsbDk engine only if that CLI becomes a limit.
  - Windows client: the usbip-win2 driver (BSD-2-Clause) in a native install.
  - macOS client: not possible (no supported virtual host controller).
- **Licence: MIT.** Never copy code from GPL projects such as usbipd-win; reading them for reference is fine.
- **Expectations:** dongles, serial adapters, HID, storage and printers work well. Webcams and audio (isochronous) work on a good LAN but not perfectly.
