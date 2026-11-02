# Panel

A titled box on the Overview, holding a `kp-mini` list.

- `kp-panel` with `kp-panel__head` (title and a short count) and an optional `kp-panel__foot` link to the full page.
- `data-tone="needs"` makes the amber Needs you panel: pending changes, rejected changes with their reason, a missing kernel module, or a missing server connection. It shows only when something needs you.
- Panels sit in `kp-panels`, a grid that stacks to one column on a phone.
- `kp-panel__body` pads content that is not a list, such as role cards. `kp-panel__note` in the header says whether the panel's setting is saved: `data-state="pending"` for "Saving…", `fault` for "Not saved: why".

## Settings

Settings pages are Panels too, one per topic (Role, Sharing), so they read like the Overview.

- `kp-settings` lists `kp-setting` rows: `kp-setting__title` and `kp-setting__help` on the left, the control in `kp-setting__control` on the right. The control wraps below the text when the row gets narrow.
- The row names the control, so the Switch or Field in it hides its own label for screen readers only (`kp-sr-only`).
- In React: `Panel` holding `SettingList` and `Setting`.
