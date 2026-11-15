# Port and Accept from for this server (design)

Status: built in the service (koppel-config, koppel, koppel-web); the UI is not moved yet. The approved Koppel prototype edits two things for this machine as a
server on the Connection page: **Port** and **Accept from** (this network only, or any network),
staged with Revert and Apply. Neither is a setting today: the network port is only the startup
option `KOPPEL_NETWORK_PORT` (else the UI port), and `KOPPEL_UI_ALLOW_FROM` covers the UI alone. The
network surface takes any caller that has credentials.

## Settings

`settings.json` gains two fields, both read as their defaults when missing:

- `networkPort`: a port from 1 to 65535, or null for what the startup options say today.
- `networkAcceptFrom`: `any` (the default, which is how the network surface behaves now) or `lan`.

`lan` uses the same test as `Ui:AllowFrom`'s `lan`: loopback, private and link-local ranges, and any
network this machine has an address on. The test is shared, not copied. A refused caller gets 403 on
the network router before Skakel sees the request, and the refusal goes to the log, as the UI's does.
Discovery (UDP) is unchanged: it only announces.

## Which value wins

- A startup option or environment variable (`KOPPEL_NETWORK_PORT`) wins over `networkPort`. The UI
  shows the field read-only and says which setting holds it, so a person is never told a saved value
  that is not the one in use. The Docker and Linux installs set it there; the Windows install does not.
- `networkAcceptFrom` has no startup option, so the saved value is always the one in use.

## When it takes effect

- **Accept from** takes effect at once: the guard reads the setting on each request.
- **Port** takes effect at the next start. The network listener is bound at start-up, and moving it
  while clients are linked would drop every link. The UI says "Applies after Koppel restarts" beside the
  saved value and the system info reports the port in use, so the two can be told apart. Clients that
  followed the server by discovery find the new port; a client with a saved address needs it changed.
- The Windows firewall rules are by program (`Package.wxs`), not by port, so a new port needs no
  installer change. A Docker or Linux install that publishes the port itself must publish the new one.

## API and push

`PATCH /api/v1/settings` takes the two fields as it takes the others, with a validation message for a
port out of range, and `SettingsChanged` carries them (so `wire.ts` and its fixture change). The port in
use is already `httpsPort` in `GET /api/v1/system/info`, which gains `networkPortFixed`: true when a
startup option or environment variable set it, so the saved one is not used.

## Tests

Unit: the patch (set, clear, leave alone, describe) and the local network test `lan` uses, which is the
UI's. Not covered yet: the 403 on a network path over HTTP, and the port a start picks, which need a
running host (contract tests, to run only with your approval).

## Not decided here

- Whether `lan` should become the default for new installs. It would be the safer default and a change
  in behaviour for any install whose clients reach it from another network.
