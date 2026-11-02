# Panel

A titled box on the Overview, holding a `kp-mini` list.

- `kp-panel` with `kp-panel__head` (title and a short count) and an optional `kp-panel__foot` link to the full page.
- `data-tone="needs"` makes the amber Needs you panel: pending changes, rejected changes with their reason, a missing kernel module, or a missing server connection. It shows only when something needs you.
- Panels sit in `kp-panels`, a grid that stacks to one column on a phone.
