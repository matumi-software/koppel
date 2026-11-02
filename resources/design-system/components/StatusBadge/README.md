# StatusBadge

Names a device's state with a word and a glyph.

- Class `kp-badge` with `data-state` set to `shared`, `attached`, `pending`, `offline` or `local`.
- Words are fixed: Shared, Attached, Pending, Offline, Not shared. Add detail after the word only when it helps ("Attached · bench-2").
- Glyphs come from CSS: a circle for Shared, a square for Attached, a stripe for Pending, a cross for Offline and an empty ring for Not shared. Never remove the glyph and never show the glyph without the word.
- `pending` means a change to this device is waiting for the server to confirm it. Do not use it for "loading".
- The consumer provides the state and, optionally, a short detail. It is display only: no hover, no click.
