# Koppel architecture

Koppel shares USB devices over the network. A machine runs as a **server** (offers the devices
plugged into it), a **client** (attaches devices from a server so they appear local), or both.
One codebase and one Docker image cover every role; the role is a setting.

## Layers

| Layer | What | Status |
|---|---|---|
| Control plane | REST for commands, a live push for confirmed state, React UI behind a password. 0.1.x: ASP.NET Core with SignalR. The Rust build (0.2.0): axum with Server-Sent Events | Done in 0.1.x; the Rust build has it, with the roles and USB behind seams (see "The UI's server side in Rust") |
| Transport | [Skakel](https://github.com/matumi-software/skakel) (MIT): discovery, pairing, OAuth client credentials, certificate pinning and handover, the remote hub and the streams. Koppel keeps the server's state and the client's credentials in its own encrypted files. The UI has its own password | Done |
| Data plane | USB/IP protocol (Linux kernel spec), in Koppel's `usb` stream on Skakel | Linux server and client work. A Windows client works through usbip-win2 (see "Real hardware" below). A Windows server shares through usbipd-win, and a real one was run to a Linux client |

## What Skakel does and what Koppel does

Since 0.1.1 the transport between Koppel machines is Skakel, a library under the MIT licence. Its wire
format is protocol version 1, described in Skakel's [`docs/protocol.md`](https://github.com/matumi-software/skakel/blob/main/docs/protocol.md): discovery,
the info endpoint, pairing, tokens and the client generation, the certificate handover, following a
server that moved, the hub's own messages and the streams. Koppel 0.1.0 spoke what that document calls
version 0, so 0.1.0 and 0.1.1 cannot talk to each other.

What Koppel adds on top, and what the rest of this document is about:

- Where Skakel's state lives: `secrets.json` (server) and `connection.json` (client), encrypted (see "Config on disk").
- Which port serves Skakel's endpoints, and in which role (see "Ports and what they serve").
- The UI over Skakel's clients, keys and pairing requests, with the confirm rule.
- Koppel's own hub message, `DevicesChanged`, and its own stream type, `usb` (see "USB data plane").
- Answering discovery only while the role is server or both (see "Finding servers and pairing").
- On the client: what the link learns is saved, and the attached devices carry on through it.

## The confirm rule

The UI never assumes a change worked.

1. The UI sends the change over REST (`PATCH /api/settings`). The server answers `202 Accepted` with no body.
2. The server writes the change to its JSON config file (temp file, flush, move over the target).
3. Only after the write lands does the server push the confirmed state over the live push (`SettingsChanged`). The Rust build pushes Server-Sent Events at `/api/events`; 0.1.x pushed over SignalR at `/api/hubs/ui`.
4. The UI shows the control as pending from step 1 until the pushed value matches what the user asked for. A rejected request (400) or no confirmation within 10 seconds puts the control back to its confirmed value with the reason.

Hand edits to the config file are picked up by a file watcher and pushed the same way (see "Config
on disk" for what happens to a bad edit). While the live push is down, every control that sends
a change is disabled, and the UI refetches over REST after reconnecting.

How the UI applies it:

- `HubPushes` (under `HubProvider`, which reads the `EventSource`) applies every pushed event to the query cache for the whole app. Pages register no handlers of their own. `nearby-servers.tsx` keeps one local handler that only ends its own "starting" state.
- Pending changes live in `PendingProvider`, above the pages, one store per domain (`pending/domains.ts`: settings, serverDevices, clientDevices, clients, connection, pairingRequests, pairing). Each pending change has an activity line. Approve and Refuse on a pairing request use the pairingRequests store like any other change. Setup is a pending change in the settings domain. Leaving a page orphans nothing.
- The provider watches the query cache and judges a pending change whenever confirmed data lands, from a push or from the refetch after a reconnect. A change that failed clears, and counts as saved, once the confirmed value equals the value that failed.
- A push cancels the queries in flight for that key before it writes to the cache, so a late fetch cannot overwrite it. The activity list is the exception: a push merges one event in by id and the fetch merges with the cache after it lands, so neither loses the other's events.
- A rejection or timeout that arrives after a change settled is ignored, and a late answer to an old send cannot settle a newer one.
- The connection form remounts on a save that was pending when confirmed, so the secret leaves the page state. A late confirm of a save that had failed does not remount it, and a changed stored connection is shown without dropping edits the person has made.
- Setup sends one `PATCH { role, setupComplete: true }`.
- Changing the password completes on the REST response, because there is no pushed value to wait for (commented in `change-password.tsx`).

Code: in Rust, `koppel-web/src/api/settings.rs` (write, then answer 202) and `koppel-web/src/push.rs`
(the push); in 0.1.x, `src/Koppel/src/Koppel/Settings/SettingsStore.cs` and `Hubs/UiHub.cs`. The UI:
`src/ui/app/src/hub/` (the `EventSource` and the pushes into the cache), `src/ui/app/src/pending/`
(pending, confirm, fail), `src/ui/app/src/settings/use-settings.ts`. The contract tests check every push
against the Rust build (`koppel-contract/tests/push.rs`); the UI has its own tests (`pnpm --filter koppel.ui test`).

## Ports and what they serve

One HTTPS port, 5240, with the server's own self-signed certificate, serves both surfaces: the UI, for the
people who run this machine, and the network surface, for other Koppel machines. `Ports:Network` moves
the network surface onto a port of its own. Which port serves what is decided by the local port the
connection arrived on, never the `Host` header. A request for a surface that is not on that port, or
that this machine's current role does not have, gets 404. Plain HTTP on a web port gets a 301 to the
same host, port and path over HTTPS, answered before TLS from the first byte of the connection
(`Web/PlainHttpRedirect.cs`); it never reaches the app. The role is read from the settings on every request, so a role change takes effect at once.
The table is `Surface.Listed` in `src/Koppel/Web/SurfaceGuard.cs`; a path not listed counts as the UI's,
so a new route is never exposed to remote clients by accident. Everything Skakel maps is under `/skakel`,
which is on the network surface.

| Port | Surface | Serves | Role |
|---|---|---|---|
| 5240 HTTPS (`Ports:Ui`) | UI | The UI (single page app), `/api/auth/*`, `/api/settings`, `/api/system/*`, `/api/activity`, the live push (`/api/events` in Rust, `/api/hubs/ui` in 0.1.x), `/healthz` | Any |
| 5240 HTTPS (`Ports:Ui`) | UI | `/api/server/*` | Server or both |
| 5240 HTTPS (`Ports:Ui`) | UI | `/api/client/*` | Client or both |
| 5240 HTTPS (`Ports:Network`) | Network | Skakel under `/skakel`: info, pairing, tokens and hello, the remote hub at `/skakel/v1/hub`, USB traffic at `/skakel/v1/streams/usb`. Koppel's own `/api/remote/*` behind a client's token | Server or both |
| 5240 UDP (`Discovery:Port`) | Network | Answers to discovery probes | Server or both |
| 3242 plain | Gateway for a USB/IP driver on this machine | Client or both, and only where `Gateway:Port` is set or on native Windows |

Settings that move them:

| Setting | Default | What |
|---|---|---|
| `Ports:Ui` | 5240 | The UI's port |
| `Ports:Network` | `Ports:Ui` | The network surface's port. Set it to split the two |
| `Discovery:Port` | `Ports:Network` | The UDP port a server answers probes on and a client sends them to. A client only finds servers that use its own discovery port |
| `Ports:Address` | Any address | The address the web ports bind to. Must be an IP address, or start-up fails naming the setting. Other Koppel machines must reach the network surface, so to keep the UI private use `Ui:AllowFrom` |
| `Ui:SetupWindowMinutes` | 10 | How long a machine without a UI password takes its first one, after it starts or after a reset. Whole minutes from 1 to 1440, or start-up fails naming the setting. See "UI sign-in" |
| `Ui:AllowFrom` | `lan`; `local` in Development | Who may use the UI, by the caller's address, as Sunshine's `origin_web_ui_allowed`. `local` is this machine only, `lan` adds the local network, `any` is everyone. A refused UI request gets 403; the network surface is not affected. The local network is loopback, 10/8, 172.16/12, 192.168/16, 169.254/16, IPv6 unique local and link-local addresses, and any network this machine has an address on (read again every 30 seconds), so a neighbour with a global IPv6 address counts. Carrier-grade NAT (100.64/10) does not. The Windows installer sets `local`, because the native app's UI is used on that machine only |
| `Gateway:Port` | 3242 on native Windows, off elsewhere | See "USB client". A compose file sets it by environment variable |
| `Gateway:Address` | `127.0.0.1` | Where the gateway listens |
| `RateLimits:TokenPerMinute`, `RateLimits:PairPerMinute` | 60, 120 | Skakel's limits on its token and pairing endpoints, per address |
| `RateLimits:AuthPerMinute` | 30 | See "Guards on the web surface" |

`GET /healthz` is on the UI port, anonymous, answers `ok` in any role and needs no header. The Docker
image's health check is Koppel itself (`dotnet Koppel.dll --healthcheck`, `Web/HealthProbe.cs`), because the port is HTTPS and the image has no curl or openssl. It calls `/healthz` on `Ports__Address` and `Ports__Ui` (any-address and unset mean loopback), then on 127.0.0.1, and accepts any certificate, since it sends nothing secret. Plain Docker only marks the container unhealthy and does not restart it; `restart:` acts on exits. Swarm and Kubernetes act on it, and with plain Docker something such as an autoheal container has to.

## Auth

The certificate, the registered clients, the tokens, revoking and rotating are Skakel's; see "Tokens" and
"Certificates" in Skakel's [`docs/protocol.md`](https://github.com/matumi-software/skakel/blob/main/docs/protocol.md). What Koppel does with them:

- **Certificate:** Skakel makes a self-signed certificate on first start, valid for ten years. Kestrel presents it on every HTTPS port, UI included (`UseSkakelCertificate`), the current one at each handshake. Its SHA-256 fingerprint shows in the server's Clients section.
- **Clients:** the server admin adds a client by name and gets a client ID and secret. Skakel takes control and format characters out of the name; a name with nothing left gets 400. Koppel sets Skakel's `KeepClientSecrets`, so the secret is kept inside the encrypted `secrets.json` and the admin can copy it again later from the client's row.
- **Revoking and rotating:** both block the client at once, everywhere: its tokens, its hub connections and its USB streams. Skakel tells the client why, and Koppel's client shows that it has to pair again.
- **Tokens:** OAuth 2.0 client credentials from Skakel, 30 minutes, over HTTPS only. Koppel's `/api/remote/*` takes them through `RequireSkakelClient()`. A token names the fixed issuer `https://skakel.invalid/`, so it works at any address or name of the server.

### Rotating the certificate and the signing key

Both are replaced from the server's Clients section through `SkakelKeys`. Each action answers 202 and is
confirmed by `ServerKeysChanged`, like any other change, and each is an activity entry.

- **Signing key:** Rotate signs new tokens with a new key at once; the one it replaced is still accepted until its tokens have expired, so no client notices.
- **Certificate, in two steps,** because clients pin it: Prepare makes the next certificate and Skakel tells the linked clients its fingerprint over the remote hub. The Clients section shows who has it. Switch presents it at once; switching before every client has it is allowed, after a warning. Cancel drops it. A restart anywhere in between changes nothing, because it is all in `secrets.json`.
- **On the client:** the link saves what it learns in `connection.json`: the next fingerprint as a second trusted one (`ClientConnection.NextServerFingerprint`), and after the switch the new pinned one. Only once that is saved does the link tell the server it has it. The same credentials with the fingerprint moved on to the one it was told about is the same server (`ClientConnection.Continues`): the link and the attached devices carry on, and `attached.json` moves to the new fingerprint.
- **A client offline the whole time** never hears of the next certificate. After the switch it reports `certificateMismatch`, as for any certificate it does not know, and must be paired again or given the new fingerprint under Configuration (the stored secret carries over when the new fingerprint is the one that server announced).

### UI sign-in

The UI on port 5240 (HTTPS) needs a password. There is one admin password and no user name.

- **First run:** whoever opens the UI first sets the password, as in Home Assistant, Jellyfin and Portainer. There is no setup code. To limit who that can be, setup is open only for a window (`Ui:SetupWindowMinutes`, 10 minutes), as Portainer does: from when Koppel starts without a password, or from a reset while it runs. While it is open, `POST /api/auth/setup` takes only `{ password }` and signs that browser in. After the window it answers 403 with "Setup has timed out", and the UI says to restart Koppel. With a password set it answers 409. `GET /api/auth/state` carries `setupOpen` and `setupUntil` (when setup closes, or closed; null once there is a password), so the first-run screen asks only for the password, or explains the timeout. While no password is set the UI asks for the state every 15 seconds, so it notices the timeout, or someone else setting the password first.
  - The log says once, at Warning, that Koppel is waiting for its first password, at `https://<machine>:<Ports:Ui>`, and until when. Nothing in it is secret. Who set the password is logged too.
  - `Ui:AllowFrom` applies to setup as to the rest of the UI, so a native Windows install (`local`) can only be set up from that machine.
  - Earlier builds wrote a setup code to `setup-code.txt`. Koppel deletes that file on start.
  - Setup is step 1 of the first-run wizard in the UI.
- **Password:** at least 10 characters, at most 128, and not "koppel" or "password" in any case. It is hashed with PBKDF2-SHA256, 600,000 iterations and a 16 byte salt, and kept with a security stamp in the encrypted `ui-auth.json`.
- **Cookie:** `koppel.ui`, HttpOnly, SameSite=Strict, Secure (the UI is HTTPS only), a session cookie that lasts 12 hours and slides while it is used, and never more than 7 days after the sign-in (the password change does not restart that). The security stamp, the session id and the 7 day cap are checked on every request. An API call that is not signed in gets 401, never a redirect.
- **Backoff:** login and password change lock a caller for one minute after 5 wrong tries, and double the lock on each further failure, up to one hour. A caller is one whole address, an IPv4-mapped IPv6 address counting as its IPv4 address, as in Skakel's limits. Not an IPv6 /64: every host on a network shares it, so one host could lock all of them out. A try is counted before the password is hashed and given back when it is right, so parallel guesses cannot all get in ahead of the lock. The lock applies before the password is looked at, so a right guess during it is refused too. Setup has nothing to guess, so it has no backoff; the rate limit and the hash slots hold it back. The table of callers holds at most 2048 entries.
- **Hash slots:** at most 2 password hashes run at once for the whole server (setup, login and password change). A caller who arrives while both are busy gets 429 with `Retry-After: 1` at once, and the try is not counted against it.
- **Failures in the log:** every failure goes to the log. The activity log gets one `signInFailed` entry each time a lock engages, so a burst of guesses cannot push the 500 other entries out.
- **Ending sessions:** a password change, a reset or signing out invalidates the cookies and closes the live push of the sessions that ended. Signing out also revokes the session id on the server, so a copy of the cookie is refused too. The revoked ids are written to the encrypted `ui-auth.json` beside the stamp, so they hold across a restart. Each one is kept until the session could no longer have been valid (7 days after its sign-in) and dropped on the next sign-out after that; the list holds at most 4096 ids, and past that the ones that end soonest go. The session that changed the password stays signed in. A push stream also ends when its cookie runs out (0.1.x: `CloseOnAuthenticationExpiration`). In the browser the sign-in gate (`AuthGate`) shows the login screen on any 401, and re-asks the server whether the session is still good when the push stream closes and on every reconnect retry, so a session that ended during an outage does not retry forever. Signing out cancels any auth check in flight and ignores rechecks until it finishes, and a failed sign-out says so.
- **Where the rule lives:** `Surface.RequiresSignIn` and `Surface.Anonymous` in `SurfaceGuard.cs`. Every `/api/*` route of the UI surface, the live push included, needs a session by default. In Rust the same table is `koppel-web/src/surface.rs`. Anonymous: `/api/auth/state`, `/api/auth/setup`, `/api/auth/login`, `/healthz` and the SPA files. The network surface never looks at the cookie; it keeps bearer tokens.
- **Forgot the password:** run the reset command. It clears the password, changes the stamp (so every session ends) and exits. A running service sees the change through its file watcher and opens the setup window again at once; otherwise the next start opens it. Then set a new password in the UI within the window. An unreadable `ui-auth.json` is replaced by the reset.

  ```sh
  docker compose exec koppel dotnet Koppel.dll --reset-password   # or: docker compose run --rm koppel --reset-password, if it is not running
  sudo koppel --reset-password                                    # Linux package
  ```

  ```powershell
  # elevated prompt
  & "C:\Program Files\Matumi\Koppel\Koppel.exe" --reset-password
  ```

### Guards on the web surface

- **Origin and header (UI surface):** an `Origin` that does not match the request's own scheme, host and port gets 403 on any `/api/*` call, the live push included. Behind a TLS proxy the scheme comes from `X-Forwarded-Proto`. A change (anything but GET or HEAD) also needs `X-Koppel-UI: 1`, otherwise 403. A page on another site cannot add that header without a CORS preflight, which this server never grants. The UI adds it in `api/http.ts` (`uiFetch`, used by every mutating fetch). The live push is a GET, so it needs no header, which an `EventSource` cannot send (0.1.x exempted its hub's WebSocket upgrade, which cannot carry one either).
- **Host header:** `AllowedHosts` stays `*` and is configurable. A DNS rebinding page is stopped by HTTPS: the browser checks Koppel's certificate against the rebinding page's own host name and the request fails, and a page that tries plain HTTP is only redirected to HTTPS at its own host name, where the same check fails. The SameSite=Strict cookie and the Origin and header checks stand behind that, not the host name.
- **Security headers** on every response: `X-Frame-Options: DENY`, `Content-Security-Policy: frame-ancestors 'none'` (only that, so it cannot break the app), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`.
- **Rate limits:** a fixed one-minute window per caller, answering 429. A caller is one whole address, an IPv4-mapped IPv6 address counting as its IPv4 address. Skakel limits its token endpoint to 60 (`RateLimits:TokenPerMinute`) and its pairing endpoints to 120 (`RateLimits:PairPerMinute`), and refuses a body over 4 KiB on them with 413. It also answers 429 to a registered client's token requests after 10 wrong secrets in a minute, from any mix of addresses. Koppel limits `/api/auth/setup`, `/login` and `/password` to 30 (`RateLimits:AuthPerMinute`), counted the same way. Nothing else has a rate limit: `/api/remote/*`, the remote hub and the streams need a valid token. Skakel's limits run first, in a middleware that `AddSkakelServer` puts ahead of everything Koppel adds, so a token or pairing request also counts when it reached the wrong port or a client-only machine and gets 404. Koppel's limit runs after the port and role check, so such a request does not use up its allowance.
- **Remote hub:** Skakel closes a connection when its token runs out, and the link makes it again 30 seconds before that, with a fresh token. Koppel shows nothing of it: no "connecting" and no entry in the activity log.
- **Connection test:** the client's Test button gets one of a fixed set of outcomes with fixed wording, never raw exception text: `ok`, `incomplete`, `unreachable`, `certificateMismatch`, `rejectedCredentials`, `clientUnknown`, `versionMismatch` (Skakel's wording, which names both sides' protocol versions), `unexpectedAnswer` and `tooManyRequests`. They are Skakel's failure kinds, mapped in `ConnectionTestResult.From`; a revoked or rotated client reads as `clientUnknown`. `unexpectedAnswer` also covers an answer over Skakel's 64 KB cap, and `tooManyRequests` a client that had too many wrong secrets in the last minute. The detail goes to the log.
- **Dev proxy:** the Vite dev server proxies `/api` to `KOPPEL_UI_URL`, default `https://127.0.0.1:5240`, the UI port, without verifying Koppel's self-signed certificate. It sets `X-Forwarded-Proto` so the Origin check passes.

### Threat model

What the design holds against:

- A stranger on the network reaching the UI: the UI needs a password, and guessing it is slowed by the backoff. Before the first password is set, though, whoever reaches the UI first during the setup window can set it and so claim the machine. `Ui:AllowFrom` limits who that can be: the local network by default, this machine only on a native Windows install. The window closes after 10 minutes (`Ui:SetupWindowMinutes`), so a machine left without a password does not wait for anyone who comes along; a restart opens it again. Set the password as soon as Koppel first starts, and on a network you do not trust, start it with `Ui:AllowFrom=local` until you have. Behind Docker Desktop every caller looks like the same address, so `Ui:AllowFrom` cannot narrow it there (see `docs/windows.md`).
- A web page in the admin's browser calling the UI: SameSite=Strict, the Origin check and the `X-Koppel-UI` header.
- A stranger reaching the network surface: it is only Skakel's info, token and pairing endpoints, which have rate limits, and the remote API, the remote hub and the USB stream, which need a token from a client registered at its current generation. The UI on the same port turns away callers outside `Ui:AllowFrom` (the local network by default) and needs the password. Plain HTTP on the port gets only a fixed redirect, before TLS.
- A stranger sending discovery packets: a server answers only a full-size probe, never with more bytes than the probe had, and a malformed or oversized datagram is dropped without stopping the answers. A probe from port 0 or from an address that is not one host (unspecified, broadcast, multicast) gets no answer, and an answer that cannot be sent is logged at debug and does not stop the next one. A client keeps a bounded list of answers (Skakel's discovery limits), and a server it hears is trusted only through its pinned certificate and the pairing code.
- A man in the middle during pairing: Skakel derives the code from the pinned certificate and both nonces, with a commitment, so it gets one guess in a million per request, and the server limits the requests. Only the opener of a request holds its poll token, so nobody else can collect the credentials.
- A revoked client: its sessions end and it cannot get a new token.

What it does not cover:

- The browser cannot tell Koppel's self-signed certificate from an attacker's. It warns the first time, and whoever accepts the warning on a network they do not trust can be shown an impostor that collects the password. The certificate's SHA-256 fingerprint is the one under Clients, so it can be checked in the browser's certificate details. On Windows the installer sets `Ui__AllowFrom=local`, so the UI answers this machine only; the port itself listens on every address, because other Koppel machines use it. The Docker Desktop compose file for a Windows client publishes the port on `127.0.0.1`; the one for a Windows server publishes it on every address, as a server must.
- `Ui:AllowFrom` and the per-caller limits on sign-in and pairing go by the caller's address as Koppel sees it; forwarded headers are not read. Behind NAT, such as port publishing that rewrites the source address, every caller arrives from one address: the local network check then lets everyone in, and one caller's wrong passwords lock out everyone else's address too.
- The gateway on 3242 is plain USB/IP with no login (see "USB client"). Whoever reaches it uses the client's access to the server. On native Windows with a loopback address only the kernel's own connections get in, and only for imports Koppel asked the driver for; elsewhere, such as a container, the port publishing is all that keeps it local.
- A registered client and the server it is paired with trust each other's USB/IP traffic: what one sends goes to the other's kernel driver (`usbip-host`, `vhci_hcd` or usbip-win2's driver) as it comes, and an attached device can be any device, a keyboard that types included. Koppel checks the bus ID and VID:PID of an import, not the traffic after it.
- Anyone who can read the keys directory can decrypt the secret files, unless `Paths:KeyCertificate` protects the key ring (see "Config on disk"). That setting is deprecated and goes in 0.2.0.
- The Linux container is privileged and runs as root, and the Windows service runs as LocalSystem. A flaw in Koppel is a flaw with that reach.

## Finding servers and pairing

Discovery, following a server that moved, and the pairing exchange with its code, its poll token and its
limits are Skakel's; see "Discovery", "Following a server" and "Pairing" in Skakel's
[`docs/protocol.md`](https://github.com/matumi-software/skakel/blob/main/docs/protocol.md). What Koppel adds:

- **Discovery:** a client browses with Skakel's `SkakelBrowser` on its discovery port (`Discovery:Port`, the HTTPS port number 5240 unless changed), keeps only servers whose app id is `koppel`, and lists them under Configuration, Servers on your network, leaving out itself. When other addresses answer with the same fingerprint, the row says so: a machine on more than one network does that, and so does one copying another server's fingerprint. A server answers probes only while its role is server or both. Koppel answers them itself (`ServerAnnouncer`), in Skakel's format, because Skakel's discovery service is on or off for the life of the host. `Discovery:Enabled=false` turns both sides off. Broadcasts stay on the local network, so a server elsewhere is entered by address.
- **Why not multicast DNS:** it worked, but it needs UDP 5353 as well, and Koppel's aim is one port number between machines.
- **Following a moved server:** the link follows its server when the saved address stops answering and discovery hears it, with a certificate it trusts, at another address. Koppel saves the new address through `ConnectionStore` like any other write, unless the person saved another connection meanwhile, so the UI hears `ConnectionChanged` and the activity log gets a `serverMoved` entry. The attached devices carry on. With `Discovery:Enabled=false`, or a server outside broadcast reach, nothing moves.
- **Pairing:** instead of copying a client ID and secret, the client shows a six-digit code and a person types it on the server's Clients page, under Waiting to pair, as with Moonlight and Sunshine. On a match the server registers the client and Skakel hands it its credentials; they go straight into `connection.json`, and `ConnectionChanged` confirms it as for any saved connection. Approve answers 202 and is confirmed by `PairingRequestsChanged` and `ClientsChanged`; a wrong code is a 400 at once. Approvals and refusals go to the activity log.
- **Replacing a connection:** if the client already has a connection to a different server, an approved pairing does not overwrite it. The client goes to a `confirm` state that shows the new server's name and fingerprint, and the person chooses Replace or Discard (`POST /api/client/pairing/confirm` to replace, `DELETE /api/client/pairing` to discard). The held credentials stay in memory until the person acts, or pairing is started again or cancelled. The `paired` state clears once the saved connection no longer matches it.
- **Endpoints:** `/api/server/pairing` for the server's UI; `/api/client/discovery` and `/api/client/pairing` for the client's UI. Pushes: `ServersDiscovered`, `PairingStatusChanged`, `PairingRequestsChanged` (an object: `requests` and `budgetExhausted`, which says the server had its fill of requests for now and is turning new ones away; only requests that got as far as a code count). The client's pairing status carries the server's fingerprint, for the confirm step. A machine that is a client only answers 404 to everything, and the client says it may be set up as a client only.
- **Saved connection:** `PUT /api/client/connection` keeps the stored secret only when the server address and fingerprint are unchanged. Otherwise the request must carry a new secret, or it is refused with 400.
- **Docker:** broadcasts do not cross Docker's bridge network, so the Linux server compose file uses the host network. On Windows, Docker Desktop's host networking does not reach the LAN: use the native install there. On Windows the installer adds the firewall rules (see Firewall below).

## USB data plane

### USB server

- **Host:** `IUsbHost` says what a host can do instead of throwing or pretending: `CanShare`, `CanUnshare`, `CanReset`, and `SharingAdvice` (the command to run on the machine that does the sharing, such as `usbipd bind`). Its methods are async and take a cancellation token. `ExportAsync(device, stream, exported, ct)` owns the relay and returns when the session ends; `exported` runs before any USB traffic, which is when Koppel answers the client's import request.
- **Linux host:** `UsbServer` polls sysfs every two seconds and applies the share choices kept in `devices.json`, which holds the bus IDs and the VID:PID each had when shared. It re-shares a device when it is plugged back in. `SysfsUsbHost` binds a device to `usbip-host`, and takes the bus ID off `match_busid` again when a bind fails. The poll survives any error from the host: it logs it and asks less often until the host answers again.
- **Pushing the list:** `UsbServer` takes each snapshot of the device list, and its sequence number, under a lock and delivers it to `Changed` handlers outside it, in order, through one pump that drops snapshots a newer one overtook. A handler that never returns holds up only the pump, never an import, a share or the poll. An import or release asks for a refresh and does not wait for it.
- **Exportable rule:** a device can be exported only when it is bound and is the VID:PID that was shared on that bus port. A bus ID is a port, so a different device plugged into the same port is unbound, shows a fault and is not shared without asking. A share whose apply fails is not saved. Bus IDs are validated (`UsbBusId`) before anything is saved; the endpoint answers 400.
- **Start-up reconciliation:** at start, Koppel shares what was saved (when `ShareOnStartup` is on and the role serves) and unbinds every other device bound to `usbip-host`, such as a share a crash left behind. A sysfs error here is logged and does not stop start-up; the poll applies the saved shares on its next pass.
- **Role change:** the surface guard serves the data plane only while the role is server or both. Leaving the server role closes the USB streams with Skakel's close code 4003, ends the remote hub connections (`LiveSessions`, through `SkakelSessions.EndAllAsync`), and unshares the devices in the background, so the settings change is confirmed to the UI without waiting for the unshare.
- **Session:** Koppel's stream type `usb`, at `/skakel/v1/streams/usb` on the network port, pinned to the server's certificate and with the client's bearer token. Skakel checks the token and that the client is registered at its current generation before the upgrade (a missing or invalid token gets 401 with a problem body), and `UsbDataPlane` then speaks plain USB/IP inside the stream's binary frames: a device list, or an import followed by the device's traffic. The type takes no query string and has no control messages or close codes of its own. The client has 10 seconds to ask for one. A session ends when its client is revoked or rotated (close code 4001 or 4002), when the role leaves the server side (4003), when either end misses pings for 30 seconds, or when Koppel stops.
- **Import:** the status check, the reset and the start of the export run under a lock per bus ID, so two imports of one device cannot race the reset. An upstream host (usbipd-win) claims the bus ID before it connects to the upstream server and releases it after the connection is closed.
- **Relay:** on import the server writes one end of a loopback socket pair to the device's `usbip_sockfd`, so the kernel serves USB from there, and relays the other end through the WebSocket. The loopback pair accepts only the kernel end's own connection. Both ends ping every 15 seconds and give up after 30 without an answer, so a client that vanished frees its device.
- **Revocation:** see "Auth". Skakel ends a client's USB streams the moment it is revoked or rotated.
- **Live list:** clients follow the server's shared devices over the remote hub, `RemoteHub` on Skakel's `SkakelHub` at `/skakel/v1/hub` (HTTPS, bearer token, pinned certificate), as Koppel's own message `DevicesChanged`. A new connection gets the list at once. The remote view shows the shared devices with their bus ID, VID:PID, name, manufacturer, speed, state and interfaces. It blanks who holds a device, since when, the serial number, the driver and any fault text: another client learns only that the device is taken.
- **Docker:** USB needs a privileged container running as root, and the host must have the modules loaded (`modprobe usbip-host` on servers, `modprobe vhci-hcd` on clients). Attached devices appear on the client's host kernel.
- **Windows:** the container uses a USB/IP driver on the Windows host over plain USB/IP inside the machine. A server shares what usbipd-win exports (`Usb:Upstream`, `UsbIpUpstreamHost`). The upstream host refreshes in the background with one deadline, so the UI reads a cached snapshot, and caps the number of devices and the bytes it accepts. `Usb:Upstream` is `host`, `host:port`, `[v6]:port` or a bare IPv6 address. A native Windows server still needs `Usb:Upstream` pointed at usbipd-win. See `docs/windows.md`.

### USB client

- **Attach:** the client opens a `usb` stream on its link, imports over it, writes its loopback end to `vhci_hcd`'s `attach` (Linux) or hands the session to usbip-win2's driver (Windows), and relays. One deadline covers the token, connecting, TLS, the stream, the import reply and the device record: 20 seconds, `Usb:AttachTimeoutSeconds`. A stall becomes a readable error on the device instead of a hang. The import reply must match the bus ID and VID:PID of the device the person picked. Bus IDs from the server are validated on receipt (`UsbBusId`) and invalid ones are dropped.
- **One link:** a saved connection has one Skakel link (`SkakelLink`), with its token and pinned connection, shared by the hub, the attaches and the gateway. A new server or new credentials get a new link; the same server at an address the person typed gets a new link and keeps its attached devices. The USB stream pings every 15 seconds and closes after 30 without an answer.
- **Fail visibly:** every background loop (link, attach, gateway) catches everything, logs it, shows the state and backs off. Failures use Skakel's fixed wording (`RemoteFailure.Describe`), for example a certificate that no longer matches the pinned fingerprint, and a revoke or rotation on the server says to pair again.
- **Attachments:** they are keyed by the client ID the server issued plus the bus ID, so two servers with the same bus ID cannot be mixed up, and a server switching to a new certificate or moving to another address keeps them. Asking to attach a device that is attached or on its way is a no-op.
- **Connection or role change:** when the saved connection changes to another server, or the role leaves client and both, Koppel detaches and stops everything first. The devices stay remembered in `attached.json`, so they attach again when the new link is up. The release runs in the background (the driver's own command can take 30 seconds), so the settings update, and the UI's confirmation, return at once; an attach that starts meanwhile waits for it. Detaching by hand releases the device first and forgets it after: if `attached.json` cannot be saved, the device is detached and shows a fault. A detach during an attach that is still in flight is noticed when the driver returns, and the driver's attachment is released.
- **Gateway:** `UsbIpGateway` runs only while the role is client or both. It is a plain USB/IP server for a USB/IP driver on this machine (see "Windows" below). It serves at most 64 sessions at once (`Gateway:MaxSessions`) and closes the rest at once. If the port cannot be bound, it tries again with a wait that grows from 5 to 60 seconds, and the reason shows as `gatewayFault` in the client's device list (the API carries it; the UI does not display it yet).
- **Native Windows:** the gateway accepts only connections owned by the kernel (PID 4, which is how usbip-win2's driver connects), looked up in the TCP table. Another process that reaches the loopback port is turned away. This applies on native Windows with an IPv4 loopback address. An IPv6 loopback address is refused outright with a warning. Where a driver is present, the gateway also serves only imports Koppel asked for: before it runs `usbip attach` it books one import for that bus ID, and the gateway accepts exactly one import for the booking. Another local user's `usbip attach`, a device list request, and the driver's own attempts to attach again after a drop are refused; Koppel attaches a remembered device again itself. Elsewhere, such as the Linux container of Docker Desktop, the connection does not come from this machine's process table, so no check applies and the compose file's port publishing is what keeps it local.
- **Stopping:** `StopAsync` detaches every attachment made through the Windows driver (`usbip attach --stop`, then `detach -p`) and stops every relay, so a Linux shutdown does not wait on them. Devices stay remembered. A `--stop` that fails because there was nothing to stop does not fail the attach or the detach.
- **Windows driver:** `UsbipWin2Driver` runs usbip-win2's `usbip.exe`. The bus ID is checked before it is passed, as `--bus-id=<id>`.

### Tested

- **Running the tests:** `dotnet test` runs everything. Skakel's own tests cover discovery, pairing, tokens, the handover and the streams; Koppel's cover what it builds on them.
- **Fakes:** the protocol encoding, the sysfs adapters against a fake `/sys`, and a full server-to-client run on real Kestrel with fake kernels on both ends (token, pinning, hub, stream, import, byte round trip). The server and client changes listed above (deadlines, revocation, the exportable rule, reconciliation, the gateway check, detach on stop) have fake-based tests, and most were also run on real hardware on 2026-11-03 (below).
- **Windows client through Docker Desktop (2026-11-03):** `usbip.exe attach -r 127.0.0.1` through the container's gateway attaches without elevation, Windows enumerates the device, Koppel shows it attached through the gateway, and Koppel's Detach removes it.
- **Not exercised on real hardware:** the attach deadline on its own (the gateway only served the kernel's connections).
- **Real kernel (2026-11-02, WSL2 6.6, software USB device):** sharing binds the device to `usbip-host`, both kernels take Koppel's sockets, and the device enumerates through both relays and TLS up to `SET_CONFIGURATION`. That step deadlocks because the test device and its attached copy sit on the same virtual controller, the only one the WSL kernel has; the same happens with no Koppel in the path. Configuration and bulk data still need a client on a second machine.
- **Real hardware (2026-11-02, first run):** a Debian thin client runs the server in Docker and shares a Logitech USB receiver. A Windows 11 client attaches it through the installed service and usbip-win2, and through Docker Desktop's gateway, and it works as keyboard and mouse there.
- **Real hardware (2026-11-03):** a Debian 13 machine runs the server in Docker and shares two Logitech receivers, a flash drive and a webcam. A native Windows 11 client attaches the receivers and the drive, and they keep working across an installer upgrade. A 256 MB file written to the drive and read back after a detach and reattach matched its checksum (about 8 MB/s). A 75 second network drop was recovered from by itself: the server freed the devices after about 50 seconds and they reattached within 2 seconds of the traffic returning. Rotating a client's secret ended its sessions at once, and stopping the client service removed the devices. A Windows 11 PC running a real usbipd-win shared a webcam to the Debian machine, which streamed it at 30 fps, so isochronous transfers cross the relay in that direction. A Logitech C270 webcam from the Linux server to the Windows client attaches but delivers no frames (#22).
- **Linux package (2026-11-03):** the amd64 `.deb` was installed, upgraded, removed and purged on a Debian 13 machine with real USB. The service started, the setup code (as first-run setup worked then) was in `setup-code.txt`, an upgrade kept the certificate and password, and purge deleted `/var/lib/koppel`.
- **Windows installer (2026-11-03):** run on a clean Windows 11 22H2 VM: fresh install, service recovery after a killed process, major upgrade, uninstall, taking over an administrator-owned data folder, and refusing one owned by a non-administrator. The rollback of a failed fresh install was not exercised.
- **Untested:** printers, and the arm64 image and `.deb` on a Raspberry Pi.

### Windows

The container uses a USB/IP driver on the Windows host over plain USB/IP inside the machine. A server
shares what usbipd-win exports; a client runs a gateway that usbip-win2 attaches from
(`Gateway:Port`, `UsbIpGateway`). See `docs/windows.md`. Installed natively, a Windows client
attaches through usbip-win2's driver, which the installer bundles; on native Windows the gateway is
on by default (port 3242, loopback). A native Windows server still needs `Usb:Upstream` pointed at
usbipd-win.

## Activity

The server keeps an activity log in memory (Koppel.Core.Activity):
sharing, plugging and unplugging, attaches and releases, client changes, settings changes, signing key
and certificate changes, and failed sign-ins, each with an outcome and, when it failed, the reason. The UI loads it from `GET /api/activity`
and follows `ActivityRecorded` over the hub, so it is the same after a reload and on every device. A subscriber that fails is logged and does not fail the code that recorded the event. It
starts empty when the process restarts. A change the UI sent shows as Waiting until the server logs
it. A settings change that changes nothing records nothing.

## Running on another machine

`src/Koppel/deploy` runs Koppel on a Debian or Ubuntu machine:

1. Copy `prepare-host.sh` to the machine and run it once with `sudo bash prepare-host.sh`. It installs Docker from Docker's signed apt repository, loads `usbip-host` and `vhci-hcd` and keeps them loaded at boot. If `modprobe` fails (Ubuntu cloud and minimal kernels), it prints the hint to install `linux-modules-extra-<kernel>`.
2. Copy `deploy/docker-compose.yml` there and run `docker compose up -d`. It pulls `matumisoftware/koppel` from Docker Hub.

The compose files pin the image tag (`matumisoftware/koppel:0.1.2`). To upgrade, edit the tag to the
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
a secret instead, with the `openssl` commands that make one. The setting is deprecated: 0.2.0 drops it, and an
install that uses it pairs again after that upgrade (see the changelog). Do not start using it.

## Config on disk

The config directory holds:

- `settings.json`: plain, may be edited by hand.
- `secrets.json`: Skakel's server state (`SkakelState`): signing key, HTTPS certificate, registered clients, and during a rotation the keys being retired and the prepared next certificate. Encrypted. Created on every machine at first start, whatever the role, because the certificate and the signing key are made then. Skakel reads it once at start and writes it after every change (`ServerSecretsStore`, Skakel's `ISkakelStore`). A file Koppel 0.1.0 wrote is carried over on the first start: the certificate, the signing key, and the clients whose secret hash Skakel reads (a client added before Koppel kept secrets in their current form is dropped, and logged). A 0.1.0 file that cannot be carried over is copied aside as `secrets.json.0.1.0-<UTC time>` and the server starts with a new certificate, a new key and no clients, which the log says.
- `connection.json` (client role): server address, pinned fingerprint, client ID and secret, and during a certificate handover the server's next fingerprint. Encrypted. Its format did not change in 0.1.1, so a client keeps its connection, and once its server runs 0.1.1 too it links as before.
- `ui-auth.json`: the UI password hash and security stamp. Encrypted.
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
Docker secret); Koppel logs a warning when it is not. `Paths:KeyCertificate` is deprecated and goes in 0.2.0,
which cannot read a key ring protected with it. Koppel 0.2 converts the secret files to its own format once (see "Config files in Rust").

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
- **What to do:** restore the keys directory from a backup (the keys directory and the config directory belong together). If the keys are really gone, the encrypted files cannot be recovered: move the unreadable ones aside (`secrets.json`, `connection.json`, `ui-auth.json`) and start again. A new `secrets.json` means a new signing key and a new certificate, so every client must be paired again. A lost `connection.json` means entering or pairing the connection again. A lost `ui-auth.json` means setting the password again, within the setup window after the start.

## Installer and Windows service

`src/Koppel/installer` builds `Koppel-Setup.exe`, which ships the Rust build (0.2.0, `koppel-windows`
below): an MSI inside a bootstrapper that installs the Visual C++ runtime first. `build.ps1` takes the
version from `[workspace.package]` in `src/Cargo.toml`, checks the vendored usbip-win2 files against
`usbip-win2/<version>/SHA256SUMS` and their Authenticode signers (and fails on any mismatch), builds the
UI from the pnpm lock file, builds `Koppel.exe` and `KoppelDesktop.exe` with `cargo build --release
--locked`, writes the notices and appends the Windows-only ones. Koppel's own programs link the C
runtime statically (`src/.cargo/config.toml`, as in Skakel), so they need no Visual C++ runtime; the
bundle still installs it because usbip-win2's `usbip.exe`, `devnode.exe` and `libusbip.dll` import
`vcruntime140.dll`, `vcruntime140_1.dll`, `msvcp140.dll` and `msvcp140_atomic_wait.dll`. 0.1.2's setup was
69 MB and installed 117 MB; the Rust build's setup is 29 MB (25 MB of it the Visual C++ runtime) and it
installs 11 MB.

- **Service:** Koppel runs as a Windows service under LocalSystem, set to start at boot. It is configured to restart after the first, second and third failure, after 5 seconds, and the count resets after a day without one. `sc failureflag \Koppel 1` (the `SetServiceFailureFlag` action) makes a stop with a non-zero exit code count as a failure too. `koppel-windows/src/service.rs` runs it on the `windows-service` crate: `Koppel.exe` asks to connect to the service manager, and when it was not started as a service (error 1063) it runs as a console program. As a service it starts in its own folder (relative `Paths:*` are read from there, as .NET did), reports Running once the ports listen, stops on Stop or Shutdown, and ends with exit code 0, or 1 when the start failed, which recovery acts on. The installer sets the service's environment to `Ui__AllowFrom=local`, so the UI answers on this machine only.
- **Tray icon:** modelled on Apollo, whose LocalSystem service launches its tray into the user's session. As a service, Koppel (`koppel-windows/src/launcher.rs`) starts `KoppelDesktop.exe --tray <ui url>` in every active session and starts it again if it ends (after a minute if it ended within 30 seconds). It runs with the session user's token from `WTSQueryUserToken`, not as LocalSystem, so the browser it opens is the user's and no LocalSystem window sits on the user's desktop. Its standard input is the read end of a pipe from the service, passed through `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` so it inherits nothing else. When the service stops or dies, the pipe closes and the tray removes its icon and exits; the service ends any tray still running after 3 seconds. `tray/mod.rs` is plain Win32 on Microsoft's `windows` crate (`Shell_NotifyIcon`, `NOTIFYICON_VERSION_4`) and adds the icon again when Explorer restarts (`TaskbarCreated`). **Shut down Koppel** sends user-defined control 200: the default service DACL gives interactive users `SERVICE_USER_DEFINED_CONTROL` but not `SERVICE_STOP`, so the service stops itself, with exit code 0, so the recovery actions do not restart it. **Restart Koppel** sends control 201: the service starts `Koppel.exe --restart-service`, which runs on as LocalSystem, waits for the service to stop and starts it; then the service stops itself the same clean way. If the helper does not start, the service logs it and keeps running. **Event log:** `event_log.rs` is a `tracing` layer that writes to the Application log under the source `Koppel`: warnings and errors from everything, and information from `koppel_windows::service` (started, stopping and why, stopped), as 0.1.x's `Logging:EventLog` did. The MSI registers the source (`util:EventSource`) with `Koppel.exe` as its message file; `koppel-bin/build.rs` gives `Koppel.exe` a message table whose messages 1 to 3 (error, warning, information) are `%1`, so Event Viewer shows each entry's text. **State:** the service writes one JSON object per line to each tray's pipe (`wire.rs`, the same lines as 0.1.x's `TrayWire`), so the tray needs no sign-in and opens no port. A `state` line carries the tooltip and whether the icon needs attention; `koppel_web::tray::TrayFeed` builds it from the settings, the UI password and setup window, the USB client and server and the connection, names and counts only. The launcher sends it when it changes (it wakes on every push the UI gets, on a session ending, and on a 5 second round that catches the setup window closing) and to each new tray. A tray skips a line it cannot read and a type it does not know, so a newer service can add messages. The attention icon is Koppel's icon with an amber dot drawn on at run time (`tray/attention.rs`). The pipe has a 64 KB buffer. **Notifications:** a `notice` line carries a title, a text and the UI page a click opens (`#clients`, `#attach`); the tray accepts only `#` and lower-case letters there, so a line cannot send the browser elsewhere. `koppel_web::tray::NoticeRules` makes them from the activity log (a lost link once until it is back, its return only after a loss was told, a failed attach as the client records it, once per reason) and from the pairing requests (each waiting request once; the ones waiting at start are not told). `KoppelSettings` has `NotifyPairing`, `NotifyLink` and `NotifyAttachFailures`, all on unless set, and `/api/system/info` says `tray: true` when Koppel runs as the service, so the UI shows them. The tray shows each with `NIF_INFO` and `NIIF_RESPECT_QUIET_TIME`, and `NIN_BALLOONUSERCLICK` opens the page. No tray runs outside the Windows service (a console, Docker, Linux).
- **Start menu entry:** `KoppelDesktop.exe --open` (`opener.rs`). `KoppelDesktop.exe` is built with `#![windows_subsystem = "windows"]`, so the shortcut and the tray open no console window, and `Koppel.exe` stays a console program for the service and for commands such as `--reset-password`. The service writes its UI address to `HKLM\SOFTWARE\Matumi\Koppel` (`UiUrl`, `ui_url.rs`), which every user can read, so the entry opens a moved port too; uninstall deletes the key. If the service is stopped, the entry asks, then runs itself again elevated as `--start-service` (UAC), which starts the service. It waits up to a minute for `/healthz` to answer, asked through WinHTTP with the certificate unchecked (`health.rs`), before it opens the browser. `--open <url>` opens another address, for trying it against a Koppel on another port. ICE43 and ICE57 are suppressed: they want an HKCU key path, which only a per-user shortcut needs.
- **Launch on the last page:** the bundle sets `LaunchTarget` to `KoppelDesktop.exe` and `LaunchArguments` to `--open`. WixStdBA shows Launch after an install, modify or repair; `KoppelTheme.xml` limits it to an install (`WixBundleAction = 6`), upgrades included. The bundle's window runs unelevated, so the browser it opens is the user's.
- **Firewall:** the MSI adds inbound rules for `Koppel.exe` through the WiX Firewall extension: TCP (HTTPS) and UDP (discovery answers), scoped to the program and not a port, so they follow a changed `Ports:*` setting. Private and Domain profiles only, never Public. The extension takes one profile per rule, so there are four rules (TCP and UDP, each for Private and Domain). They belong to the service's component: removed on uninstall, kept on repair and upgrade. This is safe on a client: the installer sets `Ui__AllowFrom=local`, and the server routes answer 404 in the client role.
- **Data folder:** `data-folder.ps1` runs as SYSTEM after the files are in. It creates `%ProgramData%\Koppel` with owner Administrators and a protected ACL (SYSTEM and Administrators). An existing folder that is a file or a link, or owned by an account that is not an administrator, fails the install with a message that says what to do (rename or delete the folder, copying keys out first if you need them). An existing folder owned by an administrator (SYSTEM, Administrators, TrustedInstaller, or an account in the local Administrators group) is taken over: its owner and ACL are reset, recursively, any link inside it is deleted (the link only, never what it points to), and a file or folder inside that no administrator owns is renamed aside (`not-admin-owned-<time>`) instead of being taken over. At start the host (`KoppelPaths.Prepare`) protects `%ProgramData%\Koppel` itself the same way, when the config and keys folders are in it. A machine where the folder is owned by an ordinary user needs it renamed or deleted once.
- **Driver:** `driver.ps1` installs usbip-win2's driver after the files and before the service starts. Its output goes to the MSI log (`msiexec /l*v`), not to a file on disk. pnputil may say a restart finishes the driver install: the installer then asks for a restart. It records the packages and the device node it installed in `%ProgramData%\Koppel\driver-state.json`, and uninstall removes only those, and only the ones Windows still lists with an original name of `usbip2_*.inf` (it reads the driver store through `pnputil /enum-drivers /format xml`, then `Get-WindowsDriver`, so a non-English Windows works). The record is used only if an administrator owns it and nobody else can write to it. A restart that pnputil asks for on uninstall is reported to the MSI too.
- **A standalone usbip-win2** (under Program Files\USBip or in the Apps list) is detected first (whatever the record says), logged, used, and left alone: Koppel never changes or removes its driver or device node. A machine with drivers but no record and no standalone usbip-win2 is treated as an earlier Koppel install and adopted. An uninstall with no record leaves the driver in place and says how to remove it by hand.
- **Failure and upgrade:** a fresh install that fails after the driver went in removes the driver again. An upgrade keeps the driver and updates it in place; the driver removal is skipped when an upgrade replaces the product. A same-version MSI replaces the installed one, which keeps a rebuilt dev installer from installing next to the last. A driver that will not come out never blocks an uninstall: the script prints a `WARNING:` with the manual `pnputil` command, and the MSI log has the output. An upgrade from an installer older than this one still runs that old installer's uninstall once, and it removes the driver by name.
- **Notices:** `installer/new-notices.ps1 -Rust` writes `THIRD-PARTY-NOTICES.txt` into the staging folder: Koppel's licence, every Rust crate in the two programs for `x86_64-pc-windows-msvc` from cargo-about (`installer/about.toml` lists the licences it accepts, and any other stops the build) with the licence texts found in each crate, and the npm packages the UI bundles. `build.ps1` appends usbip-win2's, CLI11's and the Visual C++ runtime's. It ships in the MSI and behind the bundle's licence link. The 0.1.x file at `src/Koppel/src/Koppel/` (NuGet, npm, .NET) stays for the C# image.

## Building and releasing

Every project restores from a committed `packages.lock.json`, and every csproj restores for `win-x64`,
`linux-x64` and `linux-arm64`, so one lock file covers every publish target. The builds restore in
locked mode **without** `-r`, then publish with `--no-restore -r <rid>`; `dotnet restore -r` fails
against the lock files. A locked build fails with NU1004 when a package changed and the lock file did
not: run `dotnet restore` on `Koppel.slnx` and `Koppel.Core.slnx` and commit the lock files.
The Dockerfile pins the Node, SDK and runtime images to exact versions, fails on an architecture it
does not know, and runs the health check. Its Node and SDK stages run on the build machine's
architecture and cross-compile (ReadyToRun included) for the target, and the runtime stage has no
`RUN`, so one x64 machine builds amd64 and arm64 without QEMU. .NET does not support running under
QEMU.

The Linux packages come from the same Dockerfile (`--target linux-packages`, run by
`installer/linux/build.ps1`). That target publishes Koppel self-contained for `linux-x64` and `linux-arm64`
and packs each with nfpm (pinned image, `installer/linux/nfpm.yaml`). Each `.deb` holds:

- the app in `/usr/lib/koppel`, with `/usr/bin/koppel` linked to it
- `koppel.service`: `Type=notify` (Koppel calls `UseSystemd`), root, `StateDirectory=koppel` with mode 0700,
  an optional `/etc/default/koppel` for settings, `Restart=always`
- `/usr/lib/modules-load.d/koppel.conf`, which loads `usbip-host` and `vhci-hcd` at boot
- scripts that load the modules and enable and (re)start the service on install and upgrade, stop it on
  remove, and delete `/var/lib/koppel` only on purge. A service that will not start is reported, not a
  failed install.

The package depends on the native libraries .NET loads. The OpenSSL and ICU package names differ between
releases, so it lists alternatives (`libssl3t64 | libssl3`, `libicu78 | ... | libicu70`). A distro with a
newer ICU needs its name added to `nfpm.yaml`.

To release:

1. Set `<Version>` in `src/Directory.Build.props`.
2. Set the image tag in the three deploy composes (`deploy/docker-compose.yml`, `docker-compose.windows-client.yml`, `docker-compose.windows-server.yml`) and the `KOPPEL_TAG` default in `src/Koppel/docker-compose.yml`.
3. Regenerate the lock files if any package changed.
4. For a new usbip-win2 version, run `installer/export-usbip-win2.ps1` (it rewrites `SHA256SUMS`; `-SumsOnly` regenerates only that), then add the new version to `$usbipWin2Pins` in `installer/build.ps1` (signer thumbprints and the hashes of the two unsigned DLLs, taken from a copy you checked by hand: `SHA256SUMS` alone could be regenerated to match a swapped file) and check which CLI11 tag its licence comes from. Run `installer/new-notices.ps1` when packages changed.
5. Add the version to `CHANGELOG.md`.
6. Build and push the image for both architectures, from `src/`, with the version and `latest`:

   ```sh
   docker buildx build -f Koppel/src/Koppel/Dockerfile --platform linux/amd64,linux/arm64 \
     -t matumisoftware/koppel:<version> -t matumisoftware/koppel:latest --push .
   ```
7. Build the Linux packages with `installer/linux/build.ps1` and publish the two `.deb` files.

From 0.2.0, the Rust build replaces steps 6 and 7 (see "Linux packages and the image in Rust"): set the
version in `src/Cargo.toml`, the image tag in the composes as in step 2, and point
`src/Koppel/docker-compose.yml`'s build at `koppel-bin/Dockerfile`. Then, from `src/`:

```sh
docker buildx build -f koppel-bin/Dockerfile --platform linux/amd64,linux/arm64,linux/arm/v6 \
  -t matumisoftware/koppel:<version> -t matumisoftware/koppel:latest --push .
```

and `src/installer/linux/build.ps1` for the three `.deb` files, and `src/Koppel/installer/build.ps1` for
`Koppel-Setup.exe`, which takes the same version. Record the image size per platform in the release notes.

## The Rust workspace

Koppel 0.2.0 is a rewrite in Rust (milestone v0.2.0, #82 to #92), built beside the C# until it does
everything 0.1.x does. `src/` is the Cargo workspace: `src/Cargo.toml`, the committed `src/Cargo.lock`,
and the build output in `src/target/`. It follows Skakel's: edition 2024, Rust 1.99, and every
third-party crate pinned in `[workspace.dependencies]` with a note on its licence.

| Crate | What |
|---|---|
| `koppel-bin` (package `koppel`) | The program. The folder is not `koppel` because that and the C# `Koppel` folder are the same folder on Windows |
| `koppel-config` | The files on disk: the config files with 0.1.x's JSON and write rules, the secret files, the server certificate, the folder hardening, and the conversion of a 0.1.x install (see "Config files in Rust") |
| `koppel-web` | The web server: axum on tokio, TLS wired by hand through tokio-rustls, HTTP/1.1 and HTTP/2 by ALPN, the plain HTTP redirect, `/healthz`, the UI, and the UI's server side: sign-in, the REST API and the live push (see "The UI's server side in Rust") |
| `koppel-contract` | Contract tests that drive a running Koppel over HTTPS (see "Contract tests" below). Not shipped |
| `koppel-usb` | The USB layer: USB/IP, the Linux kernel's drivers through sysfs, usbip-win2, usbipd-win, and the server and client roles over a stream to the other machine (see "USB in Rust" below) |
| `koppel-windows` | The Windows app: the service, the event log, the tray launcher, and `koppel-desktop` (installed as `KoppelDesktop.exe`), the tray icon and the Start menu entry (see "Installer and Windows service" above). Windows only; elsewhere it builds to an empty library and a program that does nothing. `koppel-bin` calls it in three places: `--restart-service`, connecting to the service manager, and the event log layer |

- **Certificate:** each handshake asks `CertificateSlot` for the current certificate, and replacing it takes effect from the next connection. Session resumption is off (no tickets, no session cache), so every connection presents the current certificate, which Skakel's handover relies on. The program presents the one in `secrets.json`. On a first start it makes one as 0.1.x did (self-signed ECDSA P-256, ten years, the machine's name and `localhost`) and keeps it there as a .NET-shaped PKCS#12 (`koppel_config::pkcs12::write`), with a token signing key, which is ECDSA P-256 as Skakel's Rust server signs. A 0.1.0 `secrets.json` is carried over, or copied aside when it cannot be.
- **UI:** `koppel-web/build.rs` reads Vite's output in `src/ui/app/dist`, gzips every file that shrinks by 5% or more, and rust-embed puts one copy of each file into the program. The gzip copy goes out as it is to a client that takes gzip (every browser) and is inflated for one that does not. Files under `assets/` have a hash in their names and are sent with `max-age=31536000, immutable`; index.html and the rest with `no-cache`. A path whose last part has no dot gets index.html, except under `/api`.
- **No UI built:** a debug build (`cargo build`, `test`, `clippy`) embeds a placeholder page and prints a warning, so the Rust side builds without Node. A release build stops with an error that says how to build the UI.

Run from `src/`:

```sh
pnpm install && pnpm --filter koppel.ui build   # the UI the program embeds (src/ui/app/dist), needed for a release build
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo test -p koppel-windows -- --ignored          # Windows: starts KoppelDesktop.exe's tray icon for a moment
cargo run -p koppel                             # https://localhost:5240, Ctrl+C stops it; set Paths__Config and Paths__Keys to a folder of your own
```

The same on Linux, in a container. The image has no clippy or rustfmt; add them with
`rustup component add clippy rustfmt` first to run those two there.

```sh
docker run --rm -v "$PWD:/repo:ro" rust:1.99.1-trixie bash -c \
  "mkdir /work && tar -C /repo --exclude=./target --exclude=node_modules -cf - . | tar -C /work -xf - && cd /work && cargo test --workspace"
```

### Linux packages and the image in Rust

`src/koppel-bin/Dockerfile` builds the Rust Koppel for Linux with Docker alone (#89, #90). It builds the UI
with pnpm in the node image, then cross-compiles static musl binaries with
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild), which uses Zig as the C compiler and
linker (ring has C and assembly), for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` and
`arm-unknown-linux-musleabihf`. That last one is ARMv6 with hardware floating point (VFPv2, Thumb-1), which
every 32-bit Raspberry Pi runs, the Zero and the Pi 1 included. Everything compiles on the build machine's
own architecture, so nothing runs under emulation, and one build serves all three platforms.

- **Pins:** every image by tag and digest (node, rust, nfpm, distroless), cargo-zigbuild and pnpm by
  version, Zig by version and checksum. The Dockerfile's header says how to bump them.
- **Why cargo-zigbuild:** `cross` runs a container per target, so inside a Docker build it needs Docker in
  Docker, and its last release is from 2023. Per-target gcc toolchains mean a musl cross compiler for each
  target to fetch and pin; Debian has none for ARMv6. One Zig covers every target.
- **Targets:** `--target binaries` writes `<arch>/koppel` (amd64, arm64, and arm for ARMv6),
  `--target linux-packages` the `.deb` files, and the last stage is the image. Cargo's registry, Zig's
  cache and `target/` are cache mounts, kept between builds.
- **Packages:** `src/installer/linux/build.ps1` builds `koppel_<version>_amd64.deb`, `_arm64.deb` and
  `_armhf.deb` (the ARMv6 binary, which is what 32-bit Raspberry Pi OS installs) into
  `src/installer/linux/bin`, with nfpm and the files beside it. The version is the workspace version in
  `src/Cargo.toml`. `-Image` also builds the image for `linux/amd64`, `linux/arm64` and `linux/arm/v6` and
  loads it into this machine's image store (Docker's containerd store, the default since Engine 29). It
  never pushes.
- **What a .deb holds:** the layout of 0.1.x, so an upgrade from it replaces the .NET files and keeps
  `/var/lib/koppel` (the first start converts the 0.1.x secret files, see "Config files in Rust"):
  `/usr/lib/koppel/koppel` with `/usr/bin/koppel` linked to it, `koppel.service`,
  `/usr/lib/modules-load.d/koppel.conf`, and the same install, remove and purge scripts as 0.1.x. It depends
  on nothing, since the binary is static. New in it: the applications menu entry (#68) and the systemd
  watchdog and hardening (#71, #72) below.
- **Applications menu (#68):** `/usr/share/applications/koppel.desktop` with Koppel's icon in
  `/usr/share/icons/hicolor` (16 to 256 px and the SVG, rendered by `pnpm design:icons` into
  `src/installer/linux/icons`). It runs `/usr/lib/koppel/koppel-open`, which opens
  `https://localhost:<port>/` through `xdg-open`, with the port from `Ports__Ui` in `/etc/default/koppel`, or
  5240. The package only suggests `xdg-utils`, so a server without a desktop gets nothing extra.
- **Watchdog (#71):** `WatchdogSec=30`. Koppel (`koppel-bin/src/systemd.rs`, on the
  [sd-notify](https://crates.io/crates/sd-notify) crate) sends `READY=1` once its ports listen,
  `STOPPING=1` when it stops, and `WATCHDOG=1` every half interval, but only when its own `/healthz`
  answers within a quarter interval. The health check goes through the router and its guard in process, on
  the same runtime as every connection, so a runtime with every worker stuck, or a lock that never comes
  free, stops the heartbeat. Without `WATCHDOG_USEC` (Docker, Windows, a console) none of it runs.
- **Hardening (#72):** still root, which the files in `/sys` need, with what Koppel does not use taken
  away: `ProtectSystem=strict` with `/var/lib/koppel` writable, no capabilities but
  `CAP_NET_BIND_SERVICE`, `/dev` limited to the usbfs nodes the device reset opens, a system call filter,
  and the rest of the usual list. `koppel.service` says for each setting left out what it would break
  (`ProtectKernelTunables` makes `/sys` read-only, `PrivateDevices` hides `/dev/bus/usb`, `PrivateUsers`
  loses root's ownership of `/sys`). `systemd-analyze security koppel` scores 2.1 (OK), down from 9.4
  (UNSAFE) for 0.1.x's unit. A config folder moved out of `/var/lib/koppel` needs a `ReadWritePaths=` for it
  in a drop-in.
- **Image (#90):** `gcr.io/distroless/static-debian13:nonroot`: root certificates, time zone data, the
  nonroot user (65532), no shell and no libc, with Koppel at `/usr/local/bin/koppel` and the `/koppel`
  volume as in 0.1.x. distroless publishes arm only as `arm/v7`, but static holds no programs, so the
  `arm/v6` image takes those files as they are (its package list then says armhf). There is no health check
  yet: the image has no shell or curl, and Koppel has no `--healthcheck` of its own as 0.1.x did.
- **Composes:** they keep naming `matumisoftware/koppel:0.1.2` and building the C# Dockerfile until 0.2.0
  is released; the image name stays and its run options do not change. `src/Koppel/src/Koppel/Dockerfile`
  and `src/Koppel/installer/linux` still build 0.1.x.

| | amd64 | arm64 | armhf, arm/v6 |
|---|---|---|---|
| Binary | 6.7 MB | 6.2 MB | 5.8 MB |
| `.deb` (0.1.2) | 3.3 MB (53.8 MB) | 3.1 MB (50.9 MB) | 2.9 MB (none) |
| Image, compressed (0.1.2) | 4.1 MB (102.1 MB) | 4.0 MB (98.0 MB) | 3.8 MB (none) |

Tested in containers: each `.deb` installs in Debian 13 of its architecture (arm64 and armhf under QEMU, the
armhf binary run as an ARM1176 CPU, the Pi Zero's, on which Debian's own ARMv7 programs stop on an illegal
instruction) and Koppel answers `/healthz` and serves the UI; the amd64 package upgrades 0.1.2 with
`/var/lib/koppel` unchanged; under systemd in a container the hardened unit starts, passes
`systemd-analyze verify`, and a Koppel frozen with `kill -STOP` is restarted by the watchdog; the image runs
on all three platforms, as nonroot and as root on a 0.1.2 volume. Not tried yet: real hardware (a Pi Zero,
sharing and attaching through the hardened unit) and a desktop's applications menu.

### The UI's server side in Rust

`koppel-web` ports 0.1.x's `Login/*`, `Web/*`, the `*Endpoints.cs` and `UiHubRelay` to axum, with the same
paths, JSON and errors, held to them by the contract tests.

| Module | What |
|---|---|
| `app.rs` | `App`: opens every config file as 0.1.x does, makes the certificate on a first start, wires the pushes, builds the router |
| `guard.rs` | The guards, in 0.1.x's order: security headers, port and role (404), `Ui:AllowFrom` (403), Origin and `X-Koppel-UI` (403), the sign-in rate limit (429), the session (401) |
| `surface.rs`, `access.rs` | What is served on which port and role, which paths are anonymous, and who `Ui:AllowFrom` takes |
| `auth/` | `ui-auth.json`, the setup window, password rules and hashes, the lockout, the hash slots, the session cookie, sign-out and the password change, `--reset-password` |
| `api/` | The REST endpoints: settings, activity, system, server, client and USB |
| `activity.rs` | The activity log, in memory, 500 entries |
| `push.rs` | The live push |
| `roles/` | The seams for #85 to #87, and what stands in for them |
| `redirect.rs`, `server.rs` | Plain HTTP answered with a 301 from the first byte, then TLS, and the dual-stack listener |

- **Settings:** `koppel-bin` reads what 0.1.x read, from the environment (`Paths__Config`) and the command line (`--Paths:Config=...`), names in any case: `Paths:Config`, `Paths:Keys`, `Ports:Ui`, `Ports:Network`, `Ports:Address`, `Ui:AllowFrom`, `Ui:SetupWindowMinutes` and `RateLimits:AuthPerMinute`, each refused at start with its name when it is not a valid value. `ASPNETCORE_ENVIRONMENT` or `DOTNET_ENVIRONMENT` set to `Development` makes `Ui:AllowFrom` default to `local`. There is no `appsettings.json`: 0.1.x's only held log levels, which `RUST_LOG` sets now. `Discovery:Port` and Skakel's `RateLimits:*` come with #85, `--healthcheck` with the image (#90).
- **Start:** the folders are prepared, 0.1.x's secrets converted, the config files opened (start-up still stops on an unusable one), then the UI port and, when set apart, the network port listen. With no `Ports:Address` a port is one dual-stack socket, or IPv4 alone where there is no IPv6.
- **Sessions:** the cookie `koppel.ui` carries the session id, the security stamp, when it was signed in and when it runs out, signed with HMAC-SHA256. The key is `ui-session.key` in the keys directory, sealed like the secret files, so sessions survive a restart; one that cannot be opened is replaced, which signs everyone out. Every request checks the signature, the stamp, that a password is set, the 12 hours, the 7 day cap and the revoked ids in `ui-auth.json`; a request in the second half of the 12 hours gets the cookie again with a later expiry. The lockout (5 tries, then a minute doubling to an hour, per whole address, at most 2048 callers), the two hash slots and the rate limit are 0.1.x's. A reset or a hand edit of `ui-auth.json` is picked up by the file's watcher: a new stamp ends every session and reopens setup when there is no password.
- **The push:** `GET /api/events` with the session cookie answers `text/event-stream`. Each event's name is 0.1.x's hub method and its data the same JSON, sent once the change is on disk: `SettingsChanged` and `ConnectionChanged` follow their files' change deliveries, so a hand edit is pushed too, and the rest are sent by whatever made the change. A comment goes out every 15 seconds. A stream that falls 256 events behind is closed rather than skipping one, and the stream ends when its session does: signing out, a password change or reset elsewhere, the cookie running out. HTTP/2 carries any number of streams over one connection, so many open tabs do not run out of connections. The UI's `EventSource` reconnects on 0.1.x's steps (0, 2, 5, 10, then 30 seconds), asks whether the session is still good on each failure, and fetches everything again when a stream opens.
- **Unknown API paths:** a signed-in request for an `/api/...` path no route has gets 404, where 0.1.2 answered index.html.
- **The seams:** `roles::ServerRole`, `ClientRole`, `UsbServer` and `UsbClient`. Until Skakel's server comes in (#85), `StoredServer` keeps the registered clients, their secrets, the signing keys and the certificates in `secrets.json`, so adding, revoking and rotating clients and keys and the certificate handover's prepare, cancel and switch work and are saved, but there are no pairing requests (approving any answers 404), no token endpoint, no `/skakel/*` and no remote hub, so `/api/remote/devices` always answers 401. The client side keeps its connection in `connection.json`; its test answers `incomplete` or `unreachable` with a line saying this build has no link, discovery finds nothing, and pairing fails at once with a reason. USB (#86, #87) answers with no devices and says sharing and attaching are not in this build, while `devices.json` and `attached.json` are still checked as 0.1.x checks them.

### Contract tests

`koppel-contract` checks that the Rust build answers like 0.1.x (#91). Its tests drive a running Koppel
from the outside, over HTTPS, the way the browser and other Koppel machines do: every REST endpoint with
its errors, sign-in with its cookie, lockout and limits, the guards on each port and role, and pairing
between two Koppels. They check status codes, field names and shapes and behaviour, not times or ids. The
crate talks HTTP only and never links Koppel's own crates.

- **Targets:** environment variables name the Koppels to test, one for most tests and a few more for states only one test may touch (a fresh machine, a closed setup window, `Ui:AllowFrom=local`, the rate limit). The crate's docs list them. Without them every test says it was skipped and passes, so `cargo test --workspace` needs no Koppel running.
- **Never an install someone uses:** the tests change settings, the password, clients and the certificate.
- **Certificate:** the first certificate a target presents is pinned for the run, and every later connection must present it or one a test pinned before switching. The name in it is not checked, since the tests reach the target as `localhost`.
- **The live push:** `koppel_contract::push` lists the ten events with their payloads, and `tests/push.rs` checks each against the Rust build's Server-Sent Events, with the stream ending on a sign-out and on a password change elsewhere. Against 0.1.x, whose push is SignalR, those tests say they were skipped.
- **What the Rust build lacks yet:** with `KOPPEL_CONTRACT_SKAKEL=0` the parts that need Skakel's endpoints or a link (tokens, `/skakel/info`, pairing between two Koppels, discovery, the connection test against a real server) say they were skipped and the rest runs. They come back with #85. `DevicesChanged` is checked only on a target that can share a device.
- **Gaps:** what needs USB hardware (sharing, attaching), a second network, or a server older than Koppel kept secrets in is not covered; the crate's tests say so where it applies.

Against the Rust build, from the repo root (Git Bash on Windows):

```sh
bash src/koppel-contract/run-against-rust.sh              # all of them
bash src/koppel-contract/run-against-rust.sh --test push  # one file
```

The script builds `koppel`, starts the same six targets as processes, each on a fresh temporary data
folder, on ports 16240 to 16246 (`KOPPEL_CONTRACT_PORT` moves them), runs the tests and stops them. On
Windows they listen on 127.0.0.1 only, so no firewall prompt comes up, and the `Ui:AllowFrom` test is
skipped, since every caller there is loopback; elsewhere the script reaches that target at the machine's
own address. `KOPPEL_CONTRACT_RELEASE=1` tests a release build.

Against Koppel 0.1.2, from the repo root with Docker running (Git Bash on Windows):

```sh
bash src/koppel-contract/run-against-0.1.2.sh            # all of them
bash src/koppel-contract/run-against-0.1.2.sh --test auth   # one file
```

The script starts six containers from `matumisoftware/koppel:0.1.2` on fresh volumes and a network of
their own, publishes them on 127.0.0.1 ports 15240 to 15246 (never 5240, where an installed Koppel
listens), runs the tests one at a time and removes everything it started, also when a test fails.
`KOPPEL_IMAGE` runs another image and `KOPPEL_CONTRACT_PORT` moves the ports.

### Config files in Rust

`koppel-config` reads and writes the same files in the same folders as 0.1.x ("Config on disk"),
with the same rules.

- **JSON:** what System.Text.Json writes with Koppel's options, byte for byte: two space indents, the
  platform's line ending, `\uXXXX` for everything outside printable ASCII and for `" & ' + < >` and the
  backtick, dates as `2026-11-09T08:30:15.1234567+00:00`, and the properties a C# record only computes
  (`isComplete`, `hasPassword`) written too. Names are read in any case and unknown ones ignored, a
  missing one takes .NET's default, and an enum is a name in any case, never a number. The tests read
  files 0.1.2 wrote and write them back unchanged.
- **Writes and pushes:** `ConfigFile` is `JsonConfigFile`: temp file, flush, replace with retries,
  stale temp files swept, start-up never replacing a file, `devices.json` and `attached.json` starting
  empty and moving a bad file aside, and changes delivered in write order with a nested change queued.
  Hand edits are found by checking the file's size and time every quarter second, which also works
  through Docker bind mounts, where file change events may not arrive.
- **Hardening:** as 0.1.x on Windows (owner Administrators, a protected ACL for SYSTEM and
  Administrators, an unelevated process keeping its own access, links refused, `%ProgramData%\Koppel`
  itself when the folders are in it) and `chmod 700` on Linux.
- **Session key:** `ui-session.key` in the keys directory, sealed the same way, signs the UI's session cookies (see "The UI's server side in Rust").
- **Secret files:** a six byte header, `KPSF`, a format version (1) and a scheme. Scheme 1, on Windows,
  is machine DPAPI over the JSON. Scheme 2, elsewhere, is AES-256-GCM with a random key in
  `koppel-secrets.key` in the keys directory, mode `0600`; the file then holds that key's id and a
  nonce before the ciphertext. Data Protection's key ring is no longer used. As before, anyone who can
  read the keys directory on Linux can decrypt the files.
- **The upgrade from 0.1.x:** at the first start, each of `secrets.json`, `connection.json` and
  `ui-auth.json` that is still a Data Protection payload is decrypted with the 0.1.x key ring (machine
  DPAPI on Windows, plain XML on Linux) and written again in the new format, with the same JSON inside.
  All are decrypted before any is written, so a failure changes nothing: the log says why, and the
  start then stops on the first file still in the old format. The key ring is left as it was. A key
  ring protected with `Paths:KeyCertificate` is not read: its files are renamed to
  `<name>.0.1-key-certificate-<UTC time>`, unchanged, and Koppel starts as a new install, so every
  client pairs again and the password is set again.
- **What carries over:** the certificate, so pinned clients keep their pin; the clients and their
  secrets; the client's connection; the UI password. The token signing key stays in `secrets.json`
  but does not need to carry over: tokens last 30 minutes and live only in a client's memory, a 0.2
  client cannot use a 0.1.x server's tokens anyway (Skakel protocol 2), and Skakel's Rust server signs
  with ECDSA P-256 where 0.1.x used RSA, so 0.2 starts with a new key. UI sessions end once: their
  cookies were protected by ASP.NET Core's own Data Protection keys in `keys/aspnet`, which 0.2 does
  not read.
- **Certificate:** 0.1.x keeps it as a password-less PKCS#12 from .NET (3DES and SHA-1, where "no
  password" is an empty BMPString without its terminator). The PKCS#12 crates refuse that form, so
  `koppel-config` reads it itself and hands the DER certificate and PKCS#8 key to rustls.

### USB in Rust

`koppel-usb` does what the C# USB layer does (`Koppel.Core.Usb` and `Koppel/Usb`), by the rules in
"USB data plane" above: the exportable rule, start-up reconciliation, the import lock per bus ID, the
attach deadline, `attached.json` and attaching again after a drop, the gateway and its checks. Those
sections describe the behaviour; this one says where it lives in Rust and what differs.

| Module | What |
|---|---|
| `protocol`, `bus_id` | The USB/IP device list and import messages, byte for byte, and `UsbBusId` |
| `host.rs` | `UsbHost`, `UsbAttacher` and `UsbIpDriver`: what a platform can do, and `Unsupported` |
| `linux` (Linux only) | `SysfsUsbHost` (usbip-host) and `SysfsVhci` (vhci-hcd) through sysfs, and the usbfs reset |
| `windows` (Windows only) | `UsbipWin2Driver`, which runs `usbip.exe`, and `WindowsTcpTable` |
| `upstream` | `UsbIpUpstreamHost` for usbipd-win, and `UpstreamAddress` (`Usb:Upstream`) |
| `stream`, `relay` | The stream to the other machine, an in-memory pair for tests, the relay and the loopback socket pair |
| `server`, `data_plane` | `UsbServer` and `UsbDataPlane` |
| `client`, `gateway`, `peer_check` | `UsbClient`, `UsbIpGateway`, and which connections the gateway serves |
| `views`, `checks`, `activity` | What the UI sees, with 0.1.x's JSON names, the USB system checks, and the activity events |

- **The stream to the other machine:** the crate does not know Skakel. A `DeviceStream` is the byte
  stream for one device: tokio's `AsyncRead` and `AsyncWrite` to read and write, and `close(reason)` to
  end it, with `CloseReason::Done`, `Stopping` (the role left the server side) or `Failed`. On the server
  the transport accepts a client's stream after its own checks and hands it to `UsbDataPlane::serve`
  with the client's ID and name. On the client the link to the server is a `Link`, a `StreamOpener`
  that also starts, stops and says its address, made by the `Links` the transport provides; the link
  reports its state and each device list through `LinkEvents`. Skakel's protocol version 2 (#85)
  implements these on its gRPC streams, one connection per device stream. The tests use the
  in-memory pair.
- **Export in two steps:** `UsbHost::export` readies the device (the kernel holds its socket, or
  usbipd-win accepted the import) and returns an `Export`; the data plane answers the import and then
  calls `Export::relay`. That is 0.1.x's `ExportAsync`, split where its `exported` callback ran.
- **Left to the transport (#85):** keeping streams out while the role is not a server (0.1.x's surface
  guard), revoking and rotating, pings, renewing the token, following a moved server and saving its
  address (`serverMoved`), and saving the certificates the server announces. The client still carries
  its remembered devices over to the server's next certificate in `attached.json`.
- **Settings and files:** the server, the data plane, the client and the gateway follow `settings.json`
  and `connection.json` through `koppel-config`'s `ConfigFile` subscriptions, and keep `devices.json`
  and `attached.json` through it. `Usb:AttachTimeoutSeconds` and the `Gateway:` settings come in as
  `ClientOptions` and `GatewayOptions`. The activity log implements `ActivitySink`.
- **The gateway's check:** on Windows with a loopback address it looks the connection up in the TCP
  table, as 0.1.x does. 0.1.x skipped that inside a Windows container (`DOTNET_RUNNING_IN_CONTAINER`);
  Koppel ships no Windows container, so the Rust build does not look.
- **Not ported:** `UsbSockets.EnableKeepAlive`, which nothing in 0.1.x called.
- **Tests:** the C# tests' cases, with fakes: a kernel on both ends, a usbip-win2 driver, batch files
  in place of `usbip.exe` (Windows only), a stand-in USB/IP server for usbipd-win, a fake `/sys` tree
  (Linux only, as the code is, and the folders have `:` in their names), and a server and a client
  joined in memory. The cases that need Skakel (tokens, pinning, revoking, the certificate handover, a
  moved server, token renewal) or the REST endpoints wait for #85 and #84. Nothing has run on real
  hardware yet (#86, #87).

## Known limits

- The Linux container is privileged and runs as root. Narrowing it needs a test on a real kernel. The Windows composes also run as root, because the volume belongs to it.
- The installer is not code signed, and there is no CI pipeline or image signing.
- The rollback of a failed fresh install was not exercised (see "Tested").
- Printers are untried, and the arm64 image and `.deb` are untested on a Raspberry Pi.
- Webcams do not stream from a Linux server to a Windows client (#22); a Windows server through usbipd-win to a Linux client does.
- The attach deadline on its own was not exercised on real hardware; it is covered by tests with fakes.
- `Common.Logging`, which the mDNS library used to pull in, is no longer in any lock file.
- The UI does not show `gatewayFault` or the live `listen` status yet.

## USB layer decisions (from the feasibility study, 2026-11-02)

- **Protocol: USB/IP**, as documented in the Linux kernel. Standard clients (`usbip attach`, usbip-win2, Docker Desktop) work against it, and we do not write our own virtual USB driver. Koppel's value sits around it: auth, discovery, UI.
- **USB traffic does not go over the push or REST.** It uses its own TCP connection; on Linux the kernel takes over the socket. The push and REST stay the control channel.
- **USB/IP has no auth or encryption.** Koppel carries it in a Skakel stream (a WebSocket over TLS) with a small relay on each side and only allows an import after an OAuth check. The raw USB/IP port is never exposed to the network.
- **Platforms:**
  - Linux server: the kernel's `usbip-host`, driven through sysfs. Docker image, privileged.
  - Linux client: the kernel's `vhci-hcd`, driven through sysfs. Attached devices appear on the host kernel, not only in the container.
  - Windows server: start by driving an installed usbipd-win through its CLI (not bundled). Our own libusb/UsbDk engine only if that CLI becomes a limit.
  - Windows client: the usbip-win2 driver (BSD-2-Clause) in a native install.
  - macOS client: not possible (no supported virtual host controller).
- **Licence: GPL-3.0-or-later** (0.1.0 and earlier: MIT). Code from compatible projects, such as usbipd-win (GPL-3.0), may come in with its notices; GPL-2.0-only code, such as the Linux kernel's, may not. See the rule in `CLAUDE.md`.
- **Expectations:** dongles, serial adapters, HID, storage and printers work well. Webcams and audio (isochronous) work on a good LAN but not perfectly.
