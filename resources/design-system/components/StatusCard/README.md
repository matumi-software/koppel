# StatusCard

The card at the bottom of the sidebar, and of the menu at phone width. It answers three questions at a glance: can changes be confirmed right now, which machine is this and which Koppel does it run, and for a client, which server it attaches from. Sign out lives here too.

- Classes: `kp-status-card` with `data-state` live, reconnecting or down, holding `kp-status-card__head` (`kp-status-card__state` and `kp-status-card__version`), `kp-status-card__body` (`kp-status-card__name` and `kp-status-card__line`s) and `kp-status-card__actions`.
- Head: the link's glyph and word, the same shapes as the phone top bar: a breathing `shared` circle and "Live", the `pending` stripe and "Reconnecting", the `fault` cross and "Disconnected". The word is a `status` region, an `alert` when the link is down.
- The version sits at the end of the head as a small `mono-sm` chip: `v0.1.1`, the product version from `/api/v1/system/info`, never the assembly version with its commit hash. It stays quiet (`text-tertiary`, a `border-default` edge) and steps up to `text-secondary` on the tinted grounds, where tertiary would drop below 4.5:1. Leave it out until it has loaded. A version that does not start with a digit, such as `dev`, is shown as it is.
- Body: the machine name in 14px 600 `text-primary`, cut with an ellipsis when it is too long for the card (the full name is its title). Under it, in `caption` size, the lines that apply:
  - reconnecting: "Changes are paused until the link is back." in `pending-text`;
  - down: "Check that lab-rack-3 is running, then retry." in `fault-text`;
  - a client's server: "Linked to lab-rack-3." in `text-secondary`, or in `pending-text` "Connecting to lab-rack-3…", "Cannot reach lab-rack-3." or "No server connection yet.";
  - a failed sign out: "Not signed out: the server answered 500." in `fault-text`, as a `status` region.
- Actions, under a 1px rule: Sign out as a ghost button with Lucide's `log-out` icon at the end of the row, and Retry as a secondary button before it while the link is down. Sign out stays enabled whatever the link: it is not a change the server has to confirm.
- Ground: `bg-inset` with a `border-default` edge and `radius-lg` while live. Reconnecting takes `pending-subtle` with a `pending` edge and rule, down takes `fault-subtle` with a `fault` edge and rule, so the state reads from across the room.
- Width: it fills the foot of the sidebar (224px inside its padding) and of the phone menu (up to 276px). Nothing in it may push the card wider: long names cut, long lines wrap.
- In React the consumer passes the link state, the machine name, the version, a client's server link and name, and the Retry and Sign out handlers, plus the last sign-out failure. `AppShell` places it in `kp-shell__foot`.
