# The UI and its server side

## The confirm rule

The UI never assumes a change worked.

1. The UI sends the change over REST (`PATCH /api/v1/settings`). The server answers `202 Accepted` with no body.
2. The server writes the change to its JSON config file (temp file, flush, move over the target).
3. Only after the write lands does the server push the confirmed state over the live push (`SettingsChanged`). The Rust build pushes Server-Sent Events at `/api/v1/events`; 0.1.x pushed over SignalR at `/api/v1/hubs/ui`.
4. The UI shows the control as pending from step 1 until the pushed value matches what the user asked for. A rejected request (400) or no confirmation within 10 seconds puts the control back to its confirmed value with the reason.

Hand edits to the config file are picked up by a file watcher and pushed the same way (see "Config
on disk" for what happens to a bad edit). While the live push is down, every control that sends
a change is disabled, and after reconnecting the new stream's opening events (see "The push") bring the pushed resources current; only what is not pushed is fetched again.

How the UI applies it:

- `LiveProvider` owns the one `EventSource` and applies every pushed event to the query cache for the whole app, through one table of event names (`live/push-table.ts`) that also gives the names to listen for. Pages register no handlers of their own. A push that is not JSON, or that its cache entry cannot take, is logged and that entry is fetched instead. The state `down` is the link having been away through every retry step; it keeps retrying, and Retry only skips the wait.
- Pending changes live in `PendingProvider`, above the pages, one store per domain (`pending/domains.ts`: settings, serverDevices, clientDevices, clients, serverKeys, connection, pairingWindow, pairingRequests, pairing). A domain names its action as a union with a `type` where it has more than one change, how an action is judged against confirmed data, and the request that sends it; `usePending(name).request(action)` marks the change pending and sends it as a `useMutation`. Each pending change has an activity line. Allow pairing and Close pairing use the pairingWindow store, settled by `PairingRequestsChanged` carrying the window open or closed; Approve and Refuse on a pairing request use the pairingRequests store like any other change. Setup is a pending change in the settings domain. Leaving a page orphans nothing.
- The provider watches the query cache and judges a pending change whenever confirmed data lands, from a push, the opening events of a stream, or the refetch of what they do not cover after a reconnect. A change that failed clears, and counts as saved, once the confirmed value equals the value that failed.
- A push cancels the queries in flight for that key before it writes to the cache, so a late fetch cannot overwrite it. The activity list is the exception: a push merges one event in by id and the fetch merges with the cache after it lands, so neither loses the other's events.
- A rejection or timeout that arrives after a change settled is ignored, and a late answer to an old send cannot settle a newer one.
- Setup sends one `PATCH { role, setupComplete: true }`. The page leaves only when the server confirms it, and so does the address: the app lands on Clients (server or both) or Connection (client) once the confirmed settings arrive, never before, and a refused setup leaves the person on the setup page.
- Every query is a factory in `src/ui/app/src/queries.ts` (`settingsOptions()` and the rest), which hooks, the live push, the pending domains and tests all take their keys from. Relative times ("seen 3 min ago", the uptime) come from `useNow`, so they move on without a refetch, and nothing refetches on a timer.
- Changing the password completes on the REST response, because there is no pushed value to wait for (see `security/security-page.tsx`).

Code: `koppel-web/src/api/settings.rs` (write, then answer 202) and `koppel-web/src/push.rs`
(the push). The UI:
`src/ui/app/src/live/` (the `EventSource` and the pushes into the cache), `src/ui/app/src/pending/`
(pending, confirm, fail), `src/ui/app/src/settings/use-settings.ts`. The contract tests check every push
against the program (`koppel/tests/contract/push.rs`); the UI has its own tests (`pnpm --filter koppel.ui test`, run in `src/ui/`).

## Pages, forms and accessibility

- **Components are Heartwood's.** The UI is built from `@matumi/ui` (`AppShell`, `PageHeader`, `Tile`, `List`, `Table`, `Dialog`, `Callout`, `SettingsList` and the rest) on `@matumi/tokens`, with lucide icons. Koppel adds no component of its own: `src/ui/app/src/koppel.css` holds its layout, the Attached colours and a few classes, all on Heartwood's tokens. Heartwood's two packages are files in `src/ui/vendor/heartwood` (its README says how to take a new release). No other UI library, CSS framework, icon set or animation library goes in, and nothing that Heartwood has a component for is built by hand.
- **Pages** are one file each, named by their address: `#overview`, `#devices` (the server's), `#remote` (the devices of every server), `#clients`, `#activity`, `#check`, `#connection`, `#security` and `#settings`. `shell/places.ts` says which a role has, and a page its role lacks opens the overview without touching the address. A page that sends changes disables them while the push link is down (`useFrame().locked`).
- **Signed-out screens** (sign in, first password, setup closed) are loaded on demand, and an `ErrorBoundary` guards both the gate and the app: a render error shows Reload (and Sign out inside the app) instead of a blank page.
- **Navigation** items are real links, so they open in a new tab. The open page lives in the address and nowhere else. A page that opens because the person went to it takes focus on its h1 and sets the tab's title; the first page of a visit leaves focus alone.
- **One live region.** `AnnouncerProvider` draws the only `role="status"`; `useAnnounce()` is how anything speaks: how a change ended, the link going away and coming back, a failed copy, the outcome of a form.
- **Adding a server** is the one change that is not pending: `POST /api/v1/client/servers` enrols with the server and answers 201, or says why not (a wrong code under the code field, an address added already under the address), and the list follows as `ServersChanged`. A server's Reconnect, Retry every and Give up after are staged in the side panel and sent together with Apply, under the confirm rule. Port is staged the same way and says "Applies after Koppel restarts"; Accept from applies at once.
- **Secrets** are held only while on show. The server key is fetched when the person presses Show, and a new secret or key is shown once.

## Testing in a browser

The UI carries Playwright, as Heartwood's usage rules ask of every app with a web UI: `@playwright/test` is a dev dependency of `koppel.ui` and is never in the bundle. Browsers are not downloaded into the project. They live once on the machine, in the folder the user-level variable `PLAYWRIGHT_BROWSERS_PATH` names (`D:esources\playwright-browsers`), put there by `npx playwright install chromium`; set `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1` in any install step that must not fetch one (the Dockerfile's UI stage does). `pnpm --filter koppel.ui e2e` starts the Vite dev server and runs the tests in `src/ui/app/e2e` in Chromium at desktop width and at phone width. The tests stand in for the service's answers with `page.route`, so no Koppel needs to run; the first one checks the first screens and that nothing scrolls sideways. A check against two real instances (as done on 2026-11-16 with the built UI) is still a manual job.

## Activity

The server keeps an activity log in memory (`koppel-app/src/activity.rs`):
sharing, plugging and unplugging, attaches and releases, client changes, settings changes, certificate
changes, and failed sign-ins, each with an outcome and, when it failed, the reason. The UI loads it from `GET /api/v1/activity`
and follows `ActivityRecorded` over the live push, so it is the same after a reload and on every device. A subscriber that fails is logged and does not fail the code that recorded the event. It
starts empty when the process restarts. A change the UI sent shows as Waiting until the server logs
it. A settings change that changes nothing records nothing.

### The UI's server side in Rust

`koppel-web` is the UI's server side on axum, held to its API by the contract tests.

**The API.** The UI's routes live under `/api/v1`, and a path names a resource: the verb is the method.
A client's secret is replaced with `POST /api/v1/server/clients/{id}/secret`, a device is attached with
`PUT` and detached with `DELETE` on `/api/v1/client/servers/{serverId}/devices/{busId}/attachment`, the pairing window is
opened with `PUT` and closed with `DELETE` on `/api/v1/server/pairing/window`, a pairing request is
approved with `POST /api/v1/server/pairing/requests/{id}/approval` and denied with `DELETE` on the
request, a client adds a server with `POST /api/v1/client/servers`, and the server
switches to its prepared certificate with `PUT /api/v1/server/keys/current-certificate`. The unversioned
`/api/...` paths are gone: the UI is embedded in the program and always matches it, and no other client is known.
(Koppel's `/api/remote/*` for other machines is the network surface's and is not part of this API.)

**Refusals are problem JSON** (RFC 9457, `application/problem+json`), written by `api/error.rs`'s `ApiError`,
which every handler returns through `Result<Json<T>, ApiError>`. The body has `type` (`about:blank`),
`title`, `status`, `detail` (a sentence the UI shows as it is), `code` (a stable word) and, for a
validation problem (400, `code: validation`), `errors`: each field with its messages. Codes in use:
`validation`, `bad_request`, `unsupported_media_type`, `unauthorized`, `forbidden`, `not_found`,
`wrong_password`, `password_set`, `no_password`, `setup_closed`, `setup_token_invalid`, `secret_not_kept`,
`rate_limited`, `locked`, `busy` and `internal`. A change that fails to save is a 500 with code `internal`
and a fixed sentence; the cause and its whole chain go to the log. The wire types of the body are
`Problem` in `generated/wire.ts`. Dates are RFC 3339 (`time::serde::rfc3339`, `Z` form) everywhere.

| Module (`koppel-web/src/`) | What |
|---|---|
| `app.rs` | `App`: the roles of `koppel-app` with the sign-in and the access rules on top, and the routers built from it |
| `routes.rs` | One router per surface (the UI and the network), each with its layers, and the dispatcher for one shared port |
| `guard.rs` | The guards as layers: security headers, the caller address check, `KOPPEL_UI_ALLOW_FROM` (403), Origin and `X-Koppel-UI` (403), the sign-in rate limit (429), the session (401) and the role gate (404) |
| `state.rs` | The state each group of handlers takes (`AuthApi`, `SettingsApi`, `ServerApi`, `ClientApi`, `PushApi`), made from the app with `FromRef` |
| `surface.rs`, `access.rs` | Which paths are the network surface's, and which role a group needs; who `KOPPEL_UI_ALLOW_FROM` takes |
| `auth/` | `ui-auth.json`, the setup window and token, password rules and hashes, the lockout and global budget (`throttle.rs`), the hash slots, the in-memory sessions and the cookie (`session.rs`), sign-out and the password change, `reset-password` |
| `api/` | The REST endpoints: `settings.rs` (settings, activity, system info and checks), `server.rs` and `client.rs` (the role's own routes), and `error.rs`, `ApiError` and the problem JSON it writes |
| `json.rs` | The JSON request body, refused with 400 or 415 as problem JSON |
| `push.rs` | The live push's stream |
| `ready.rs` | `/readyz` |
| `tls.rs`, `redirect.rs`, `server.rs` | The certificate the ports present, plain HTTP answered with a 301 from the first byte, then TLS, and the dual-stack listener |
| `ui.rs` | The built UI, embedded at compile time |
| `wire.rs` | Test only: writes the TypeScript types of the wire (`src/ui/app/src/api/generated/wire.ts`) from the Rust types, fails when the checked-in file differs, and reads the JSON fixtures in `src/ui/app/src/test/wire/` through the Rust types; see "Wire types generated from the Rust DTOs" in `decisions.md` |

What the routes talk to is `koppel-app`, below the web server: the activity log (500 entries, in memory,
`activity.rs`), the push bus (`push.rs`), the certificate slot, and the roles in `roles/`, the server and
client roles on Skakel (`roles/skakel/`) and the USB layer behind them (`roles/usb.rs`); see
["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust).

- **Settings:** one typed `Settings` struct in `koppel/src/config.rs`, built once at start by clap from options and `KOPPEL_` environment variables, with 0.1.x's names kept as deprecated aliases. A value that is not valid, an unknown option, and a misspelt setting are all caught before anything starts (see "Settings" in [config.md](config.md)). Log levels come from `RUST_LOG`. The commands are `serve` (the default), `reset-password` and `healthcheck`, and `koppel --version` names the git commit and the build date.
- **Start:** the folders are prepared, 0.1.x's secrets converted, the config files opened (start-up still stops on an unusable one), then the UI port and, when set apart, the network port listen. With no `KOPPEL_LISTEN_ADDRESS` a port is one dual-stack socket, or IPv4 alone where there is no IPv6.
- **Sessions:** the cookie `__Host-koppel-session` carries an opaque 256-bit id, and the session behind it is in memory (`koppel-web/src/auth/session.rs`): nothing is written to disk, so a restart signs everyone out. Every request looks the id up and checks that a password is set, the 12 hours without use and the 7 day cap; a request in the second half of the 12 hours extends the session. Each session owns a `CancellationToken` that closes its live push when it ends. The lockout (5 tries, then a minute doubling to an hour, per IPv4 address or IPv6 /64, at most 2048 callers, and a global budget), the two hash slots and the rate limit are described under "UI sign-in" in `docs/security.md`. A reset or a hand edit of `ui-auth.json` is picked up by the file's watcher: a password this server did not write ends every session and reopens setup when there is no password.
- **The push:** `GET /api/v1/events` with the session cookie answers `text/event-stream`. Each event's name and JSON are 0.1.x's, sent once the change is on disk: `SettingsChanged` and `ServersChanged` follow their files' change deliveries, so a hand edit is pushed too, and the rest are sent by whatever made the change. A new stream starts with the current state of each resource the machine's role has, as the same events under the same names and in a fixed order: `SettingsChanged`, then for a server or both `ClientsChanged`, `ServerKeysChanged`, `PairingRequestsChanged`, `EnrollmentChanged`, `DevicesChanged`, then for a client or both `ServersChanged`, `ServersDiscovered`, `ClientDevicesChanged`. That is an addition to what a stream carries, with no new name or payload. The stream subscribes before it reads the state, so a change between the two arrives again as an event at least as new, and the GET-then-stream race is gone. The activity feed is not state and has no snapshot (the UI reads it with `GET /api/v1/activity`). A comment goes out every 15 seconds. A stream that falls 256 events behind is closed rather than skipping one, and the stream ends when its session does (its token is cancelled): signing out, a password change or reset elsewhere, the session running out, or the server stopping (every request carries a shutdown token from the accept loop, so a stop does not wait for an open tab). HTTP/2 carries any number of streams over one connection, so many open tabs do not run out of connections. The UI's `EventSource` reconnects on 0.1.x's steps (0, 2, 5, 10, then 30 seconds), a first connect that failed included, asks whether the session is still good on each failure, and, when a stream opens after a failure, fetches only the queries the opening events do not cover (the activity feed, the system info and checks, the sign-in state); the first open needs no fetch.
- **Unknown API paths:** a signed-in request for an `/api/v1/...` path no route has gets 404, where 0.1.2 answered index.html.
- **The roles:** `SkakelServerRole`, `SkakelClientRole`, `UsbServerRole` and `UsbClientRole` are concrete types, which the endpoints and the push talk to with 0.1.x's JSON (see ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust)). There is one implementation of each, so there is no trait between. The device and system-check views are koppel-usb's own types, serialised as they are, and so are the activity log's entity, reason and outcome.
