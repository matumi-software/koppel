# The UI and its server side

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

- `LiveProvider` owns the one `EventSource` and applies every pushed event to the query cache for the whole app, through one table of event names (`live/push-table.ts`) that also gives the names to listen for. Pages register no handlers of their own. A push that is not JSON, or that its cache entry cannot take, is logged and that entry is fetched instead. The state `down` is the link having been away through every retry step; it keeps retrying, and Retry only skips the wait.
- Pending changes live in `PendingProvider`, above the pages, one store per domain (`pending/domains.ts`: settings, serverDevices, clientDevices, clients, connection, pairingWindow, pairingRequests, pairing). Each pending change has an activity line. Allow pairing and Close pairing use the pairingWindow store, settled by `PairingRequestsChanged` carrying the window open or closed; Approve and Refuse on a pairing request use the pairingRequests store like any other change. Setup is a pending change in the settings domain. Leaving a page orphans nothing.
- The provider watches the query cache and judges a pending change whenever confirmed data lands, from a push or from the refetch after a reconnect. A change that failed clears, and counts as saved, once the confirmed value equals the value that failed.
- A push cancels the queries in flight for that key before it writes to the cache, so a late fetch cannot overwrite it. The activity list is the exception: a push merges one event in by id and the fetch merges with the cache after it lands, so neither loses the other's events.
- A rejection or timeout that arrives after a change settled is ignored, and a late answer to an old send cannot settle a newer one.
- The connection form remounts on a save that was pending when confirmed, so the secret leaves the page state. A late confirm of a save that had failed does not remount it, and a changed stored connection is shown without dropping edits the person has made.
- Setup sends one `PATCH { role, setupComplete: true }`.
- Changing the password completes on the REST response, because there is no pushed value to wait for (commented in `change-password.tsx`).

Code: `koppel-web/src/api/settings.rs` (write, then answer 202) and `koppel-web/src/push.rs`
(the push). The UI:
`src/ui/app/src/live/` (the `EventSource` and the pushes into the cache), `src/ui/app/src/pending/`
(pending, confirm, fail), `src/ui/app/src/settings/use-settings.ts`. The contract tests check every push
against the Rust build (`koppel-contract/tests/push.rs`); the UI has its own tests (`pnpm --filter koppel.ui test`, run in `src/ui/`).

## Activity

The server keeps an activity log in memory (`koppel-web/src/activity.rs`):
sharing, plugging and unplugging, attaches and releases, client changes, settings changes, certificate
changes, and failed sign-ins, each with an outcome and, when it failed, the reason. The UI loads it from `GET /api/activity`
and follows `ActivityRecorded` over the live push, so it is the same after a reload and on every device. A subscriber that fails is logged and does not fail the code that recorded the event. It
starts empty when the process restarts. A change the UI sent shows as Waiting until the server logs
it. A settings change that changes nothing records nothing.

### The UI's server side in Rust

`koppel-web` is the UI's server side on axum, with 0.1.x's paths, JSON and errors, held to them by the
contract tests.

| Module | What |
|---|---|
| `app.rs` | `App`: opens every config file as 0.1.x does, makes the certificate on a first start, wires the pushes, builds the router |
| `guard.rs` | The guards, in 0.1.x's order: security headers, port and role (404), `Ui:AllowFrom` (403), Origin and `X-Koppel-UI` (403), the sign-in rate limit (429), the session (401) |
| `surface.rs`, `access.rs` | What is served on which port and role, which paths are anonymous, and who `Ui:AllowFrom` takes |
| `auth/` | `ui-auth.json`, the setup window, password rules and hashes, the lockout, the hash slots, the session cookie, sign-out and the password change, `--reset-password` |
| `api/` | The REST endpoints: settings, activity, system, server, client and USB |
| `activity.rs` | The activity log, in memory, 500 entries |
| `push.rs` | The live push |
| `roles/` | The seams the endpoints talk to, the server and client roles on Skakel (`roles/skakel/`) and the USB layer behind them (`roles/usb.rs`); see ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust) |
| `redirect.rs`, `server.rs` | Plain HTTP answered with a 301 from the first byte, then TLS, and the dual-stack listener |

- **Settings:** `koppel` reads what 0.1.x read, from the environment (`Paths__Config`) and the command line (`--Paths:Config=...`), names in any case: `Paths:Config`, `Paths:Keys`, `Ports:Ui`, `Ports:Network`, `Ports:Address`, `Ui:AllowFrom`, `Ui:SetupWindowMinutes` and `RateLimits:AuthPerMinute`, each refused at start with its name when it is not a valid value. `ASPNETCORE_ENVIRONMENT` or `DOTNET_ENVIRONMENT` set to `Development` makes `Ui:AllowFrom` default to `local`. Log levels come from `RUST_LOG`. With the roles come `Discovery:Enabled`, `Discovery:Port`, `RateLimits:TokenPerMinute`, `RateLimits:PairPerMinute`, `Usb:SysRoot`, `Usb:Upstream`, `Usb:UsbipPath`, `Usb:AttachTimeoutSeconds`, `Gateway:Port`, `Gateway:Address` and `Gateway:MaxSessions`, as 0.1.x read them. `--healthcheck` comes with the image (#90).
- **Start:** the folders are prepared, 0.1.x's secrets converted, the config files opened (start-up still stops on an unusable one), then the UI port and, when set apart, the network port listen. With no `Ports:Address` a port is one dual-stack socket, or IPv4 alone where there is no IPv6.
- **Sessions:** the cookie `koppel.ui` carries the session id, the security stamp, when it was signed in and when it runs out, signed with HMAC-SHA256. The key is `ui-session.key` in the keys directory, sealed like the secret files, so sessions survive a restart; one that cannot be opened is replaced, which signs everyone out. Every request checks the signature, the stamp, that a password is set, the 12 hours, the 7 day cap and the revoked ids in `ui-auth.json`; a request in the second half of the 12 hours gets the cookie again with a later expiry. The lockout (5 tries, then a minute doubling to an hour, per whole address, at most 2048 callers), the two hash slots and the rate limit are 0.1.x's. A reset or a hand edit of `ui-auth.json` is picked up by the file's watcher: a new stamp ends every session and reopens setup when there is no password.
- **The push:** `GET /api/events` with the session cookie answers `text/event-stream`. Each event's name and JSON are 0.1.x's, sent once the change is on disk: `SettingsChanged` and `ConnectionChanged` follow their files' change deliveries, so a hand edit is pushed too, and the rest are sent by whatever made the change. A comment goes out every 15 seconds. A stream that falls 256 events behind is closed rather than skipping one, and the stream ends when its session does: signing out, a password change or reset elsewhere, the cookie running out. HTTP/2 carries any number of streams over one connection, so many open tabs do not run out of connections. The UI's `EventSource` reconnects on 0.1.x's steps (0, 2, 5, 10, then 30 seconds), a first connect that failed included, asks whether the session is still good on each failure, and fetches everything again when a stream opens after a failure (the first open needs no fetch).
- **Unknown API paths:** a signed-in request for an `/api/...` path no route has gets 404, where 0.1.2 answered index.html.
- **The seams:** `roles::ServerRole`, `ClientRole`, `UsbServer` and `UsbClient`, which the endpoints and the push talk to with 0.1.x's JSON. `SkakelServerRole`, `SkakelClientRole`, `UsbServerRole` and `UsbClientRole` fill them (see ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust)).
