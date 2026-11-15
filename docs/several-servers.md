# Several servers on one client (design)

Status: built in the service (koppel-config, koppel-usb, koppel-app, koppel-web); the UI is not moved yet. The approved UI is the Koppel prototype (the Heartwood design canvas and the live prototype), and everything it shows is in scope. A client today holds one server: one saved connection, one link, one
device list and one `attached.json`. The redesign lets a client add any number of servers, each with
its own link, and attach devices from all of them at once.

## What a person sees

- Connection lists the servers this computer attaches from, each with its link state and round trip.
  "Add a server" takes an address and either a pairing code or a server key (Skakel 0.3.1 enrolment).
  Each row can be removed, with a confirmation.
- Remote devices lists the devices of every server together, each saying which server it is on.
  Attaching, detaching and the history work per device as they do now.

## The saved connections

`connection.json` becomes a list of servers. Each entry is what the single connection holds today
(address, pinned fingerprint, client ID, secret, next fingerprint) plus an `id` Koppel makes (such as
`server-3fa9c01b2e`), a `name` the server gave itself, and `linkedWith` (`code`, `key` or `pairing`).

A file that has the old flat fields is read as a list of one, with a new `id`, and written as a list the
next time anything is saved. `koppel-upgrade` keeps its 0.1.x conversion, which makes the flat form, so
that conversion's output goes through the same reading. Nothing is deleted at upgrade.

## The USB client

One actor owns a link, a device list and the slots of one server. That is kept as it is. A new
`UsbClients` holds one of those per saved server and is what the rest of Koppel talks to:

- Each one gets its own view of its server's entry as its `Connection`, and its own part of
  `attached.json`. `attached.json` becomes `{ "servers": { "<id>": { bus_ids, vid_pids } } }`; the old
  top-level `serverFingerprint`, `busIds` and `vidPids` become the first server's part, matched by
  fingerprint.
- Adding a server starts an actor, and removing one stops it, which detaches what it had attached.
- A device is named by its server and its bus ID, since two servers can offer the same bus ID. Every
  REST route that names a device names the server too: `PUT` and `DELETE` on
  `/api/v1/client/servers/{serverId}/devices/{busId}/attachment`.
- What the client shows (`ClientDevices`) becomes a list of servers, each with its link state, its
  message and its devices, plus the settings that are the machine's rather than a server's (the driver
  is missing, the gateway).

### What is machine-wide and must not be shared by accident

- The kernel's virtual ports (and the Windows driver's) are one pool. Each actor asks the platform for
  a free port when it attaches, so attaches from different actors must not race for the same one: they
  go through one lock in the platform wrapper, and a test attaches from two actors at once.
- The USB/IP gateway is one listener for a driver such as usbip-win2, and the driver names a device
  only by a bus ID. The gateway serves an import from the first server, in the order they were added,
  that is offering that bus ID. The UI shows the `usbip attach` command only for a device whose bus ID
  no earlier server also offers.

## The API and the push

- `GET /api/v1/client/servers` is the list without secrets. `POST /api/v1/client/servers` takes
  `{ address, code }` or `{ address, key }` and an optional `name`, enrols with Skakel and saves the
  server. It answers 201 with the new server once it is saved, or says why it was not (422 when the server
  refused, 409 when it is added already, 400 for a bad address, code or key): enrolling needs an answer
  the person can read, which a 202 could not carry. The server also shows in `ServersChanged`.
  `DELETE .../servers/{id}` removes one and releases its devices. `POST .../servers/{id}/test` is the
  connection test for that server.
- `ConnectionChanged` becomes `ServersChanged` (the list). `ClientDevicesChanged` carries the new shape.
- Discovery stays, to fill the address; the pairing routes (`/client/pairing`) are removed once the UI
  no longer uses them.

## Per-server link settings

The approved prototype's Connection page edits, for each server, **Reconnect** (on or off), **Retry every**
(seconds) and **Give up after** (seconds), staged with Revert and Apply, and for this machine as a server
its **Port** and **Accept from** (this network only, or any network). Nothing in the service reads these
today, so they are part of this change:

- Each saved server gets `reconnect` (true), `retryEverySeconds` (30) and `giveUpAfterSeconds` (10), kept in
  its `connection.json` entry and read as those defaults when missing. The link takes them: with reconnect
  off it does not try again after it drops, otherwise it tries every `retryEverySeconds`, and a connect
  attempt that takes longer than `giveUpAfterSeconds` is abandoned.
- `PATCH /api/v1/client/servers/{id}` changes them, under the confirm rule: the value shows in
  `ServersChanged` once it is written. A change to a link setting makes that server's link again.
- What is enforced today: **Give up after** is the time one request of the link may take. **Reconnect** off
  ends the link at its first failure (Koppel stops it), and turning it on again makes a new link.
  **Retry every** is the wait between tries after a failure (Skakel 0.3.2's `retry_interval`: a random
  time from half of it to all of it, never under a second).
- Whether this machine's port and "accept from" are settings the service already has is checked against
  `docs/config.md` before anything is added for them; if they are not, they become settings in
  `settings.json` and need a restart of the network surface, which the UI says.

## Tests

`koppel-usb` already drives its client with a fake platform and fake links. New tests there: two
servers with the same bus ID attach and detach independently, removing a server detaches only its
devices, two actors attach at once without sharing a port, and the old `attached.json` and
`connection.json` read as one server. They do not start Koppel and touch no hardware.
