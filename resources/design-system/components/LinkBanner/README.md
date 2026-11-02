# LinkBanner

Shows whether the live link to the server is up, so the user knows whether changes can be confirmed.

- Class `kp-link` with `kp-link__dot`, a short state word and a `kp-link__detail` sentence.
- `data-state` follows the SignalR connection: `live` (connected), `reconnecting` (automatic reconnect in progress) or `down` (gave up, or the server refused the token).
- `live` sits quietly in the app header: a breathing `shared` dot and "Live". `reconnecting` and `down` become a full-width band directly under the header.
- While the link is not `live`, every control that sends a change is disabled, and pending controls stay pending. After reconnecting, refetch the state over REST before re-enabling anything.
- Copy: "Reconnecting. Changes are paused until the link is back." and "Disconnected from lab-rack-3. Check that the server is running, then retry."
- In React it reads the connection state from the shared hub provider; the consumer provides only the server name and an optional retry handler for `down`.
