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
- Pending changes live in `PendingProvider`, above the pages, one store per domain (`pending/domains.ts`: settings, serverDevices, clientDevices, clients, serverKeys, connection, pairingWindow, pairingRequests, pairing). A domain names its action as a union with a `type` where it has more than one change, how an action is judged against confirmed data, and the request that sends it; `usePending(name).request(action)` marks the change pending and sends it as a `useMutation`. Each pending change has an activity line. Allow pairing and Close pairing use the pairingWindow store, settled by `PairingRequestsChanged` carrying the window open or closed; Approve and Refuse on a pairing request use the pairingRequests store like any other change. Setup is a pending change in the settings domain. Leaving a page orphans nothing.
- The provider watches the query cache and judges a pending change whenever confirmed data lands, from a push or from the refetch after a reconnect. A change that failed clears, and counts as saved, once the confirmed value equals the value that failed.
- A push cancels the queries in flight for that key before it writes to the cache, so a late fetch cannot overwrite it. The activity list is the exception: a push merges one event in by id and the fetch merges with the cache after it lands, so neither loses the other's events.
- A rejection or timeout that arrives after a change settled is ignored, and a late answer to an old send cannot settle a newer one.
- The connection form remounts on a save that was pending when confirmed, so the secret leaves the page state. A late confirm of a save that had failed does not remount it, and a changed stored connection is shown without dropping edits the person has made.
- Setup sends one `PATCH { role, setupComplete: true }`. The page leaves only when the server confirms it, and so does the address: the app lands on Clients (server or both) or Configuration (client) once the confirmed settings arrive, never before, and a refused setup leaves the person on the setup page.
- Every query is a factory in `src/ui/app/src/queries.ts` (`settingsOptions()` and the rest), which hooks, the live push, the pending domains and tests all take their keys from. Relative times ("seen 3 min ago", the uptime) come from `useNow`, so they move on without a refetch, and nothing refetches on a timer.
- Changing the password completes on the REST response, because there is no pushed value to wait for (commented in `change-password.tsx`).

Code: `koppel-web/src/api/settings.rs` (write, then answer 202) and `koppel-web/src/push.rs`
(the push). The UI:
`src/ui/app/src/live/` (the `EventSource` and the pushes into the cache), `src/ui/app/src/pending/`
(pending, confirm, fail), `src/ui/app/src/settings/use-settings.ts`. The contract tests check every push
against the program (`koppel/tests/contract/push.rs`); the UI has its own tests (`pnpm --filter koppel.ui test`, run in `src/ui/`).

## Pages, forms and accessibility

- **Primitives.** A page is built from `@koppel/react-ui`: `Card`, `Stack` and `Row` for layout, `Note`, `Caption` and `SectionHeading` for text, `Drawer` (with `DrawerSection`, `Holder`, `KeyValues` and `Chips`) for the side panel, `LabeledValue` for a value that is not an input, `FirstRunShell` and `PageHead` for the pages before the app. The app sets no inline style and no layout class of its own; what it needs that the library lacks goes into the library and `resources/design-system/components/` (a README per primitive) first.
- **Signed-out screens** (sign in, first password, setup closed) are loaded on demand, and an `ErrorBoundary` guards both the gate and the app: a render error shows Reload (and Sign out inside the app) instead of a blank page.
- **Navigation** items are real links to `#devices` and so on, so they open in a new tab. The open page lives in the address and nowhere else. A page that opens because the person went to it takes focus on its h1 and sets the tab's title (`Devices - Koppel`); the first page of a visit leaves focus alone. A skip link jumps past the navigation.
- **One live region.** `AnnouncerProvider` draws the only `role="status"`; `useAnnounce()` is how anything speaks: how a change ended (the pending store says "Role: both. Saved." or "... could not be saved: why"), the link going away and coming back, a failed copy, the outcome of a form. Rows, field help, the status card and the activity feed are plain text. react-aria keeps a separate off-screen announcer for selection changes, which is not ours.
- **Forms** are react-aria `Form`s. A field's rules (`validate`, `isRequired`) are checked when the form is sent and the message takes the help's place; the server's refusal goes to `validationErrors` under the field it is about and clears when the person edits it. The send button stays enabled so the reason is never hidden. The password rules are in `auth/password-rules.ts`, the server's own (`koppel-web/src/auth/password.rs`), used by the first password and the password change; anything the server still refuses is shown as it says. Each form sends through a `useMutation`.
- **The connection form** keeps only what the person changed. Everything else is the stored connection as it is now, so a connection that changes underneath shows in the fields nobody touched and never overwrites one being typed in; a confirmed save remounts it, which drops the secret.
- **Secrets** are held only while on show: Hide drops a revealed secret, and so does a copy made while it was masked.

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
| `app.rs` | `App`: opens every config file as 0.1.x does, makes the certificate on a first start, wires the pushes, builds the routers |
| `routes.rs` | One router per surface (the UI and the network), each with its layers, and the dispatcher for one shared port |
| `guard.rs` | The guards as layers: security headers, the caller address check, `KOPPEL_UI_ALLOW_FROM` (403), Origin and `X-Koppel-UI` (403), the sign-in rate limit (429), the session (401) and the role gate (404) |
| `state.rs` | The state each group of handlers takes (`AuthApi`, `SettingsApi`, `ServerApi`, `ClientApi`, `PushApi`), made from the app with `FromRef` |
| `surface.rs`, `access.rs` | Which paths are the network surface's, and which role a group needs; who `KOPPEL_UI_ALLOW_FROM` takes |
| `auth/` | `ui-auth.json`, the setup window, password rules and hashes, the lockout, the hash slots, the session cookie, sign-out and the password change, `reset-password` |
| `api/` | The REST endpoints: settings, activity, system, server, client and USB |
| `activity.rs` | The activity log, in memory, 500 entries |
| `push.rs` | The live push |
| `roles/` | The seams the endpoints talk to, the server and client roles on Skakel (`roles/skakel/`) and the USB layer behind them (`roles/usb.rs`); see ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust) |
| `redirect.rs`, `server.rs` | Plain HTTP answered with a 301 from the first byte, then TLS, and the dual-stack listener |

- **Settings:** one typed `Settings` struct in `koppel/src/config.rs`, built once at start by clap from options and `KOPPEL_` environment variables, with 0.1.x's names kept as deprecated aliases. A value that is not valid, an unknown option, and a misspelt setting are all caught before anything starts (see "Settings" in [config.md](config.md)). Log levels come from `RUST_LOG`. The commands are `serve` (the default), `reset-password` and `healthcheck`, and `koppel --version` names the git commit and the build date.
- **Start:** the folders are prepared, 0.1.x's secrets converted, the config files opened (start-up still stops on an unusable one), then the UI port and, when set apart, the network port listen. With no `KOPPEL_LISTEN_ADDRESS` a port is one dual-stack socket, or IPv4 alone where there is no IPv6.
- **Sessions:** the cookie `koppel.ui` carries the session id, the security stamp, when it was signed in and when it runs out, signed with HMAC-SHA256. The key is `ui-session.key` in the keys directory, sealed like the secret files, so sessions survive a restart; one that cannot be opened is replaced, which signs everyone out. Every request checks the signature, the stamp, that a password is set, the 12 hours, the 7 day cap and the revoked ids in `ui-auth.json`; a request in the second half of the 12 hours gets the cookie again with a later expiry. The lockout (5 tries, then a minute doubling to an hour, per whole address, at most 2048 callers), the two hash slots and the rate limit are 0.1.x's. A reset or a hand edit of `ui-auth.json` is picked up by the file's watcher: a new stamp ends every session and reopens setup when there is no password.
- **The push:** `GET /api/events` with the session cookie answers `text/event-stream`. Each event's name and JSON are 0.1.x's, sent once the change is on disk: `SettingsChanged` and `ConnectionChanged` follow their files' change deliveries, so a hand edit is pushed too, and the rest are sent by whatever made the change. A comment goes out every 15 seconds. A stream that falls 256 events behind is closed rather than skipping one, and the stream ends when its session does: signing out, a password change or reset elsewhere, the cookie running out. HTTP/2 carries any number of streams over one connection, so many open tabs do not run out of connections. The UI's `EventSource` reconnects on 0.1.x's steps (0, 2, 5, 10, then 30 seconds), a first connect that failed included, asks whether the session is still good on each failure, and fetches everything again when a stream opens after a failure (the first open needs no fetch).
- **Unknown API paths:** a signed-in request for an `/api/...` path no route has gets 404, where 0.1.2 answered index.html.
- **The roles:** `SkakelServerRole`, `SkakelClientRole`, `UsbServerRole` and `UsbClientRole` are concrete types, which the endpoints and the push talk to with 0.1.x's JSON (see ["The roles on Skakel in Rust"](skakel.md#the-roles-on-skakel-in-rust)). There is one implementation of each, so there is no trait between. The device and system-check views are koppel-usb's own types, serialised as they are, and so are the activity log's entity, reason and outcome.
