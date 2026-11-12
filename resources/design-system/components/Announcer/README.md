# Announcer

The one live region of the app, kept out of sight (`kp-announcer`).

- Everything a screen reader needs to be told goes through it with `useAnnounce()`: how a change ended ("Share saved devices on startup: on. Saved."), the link going away and coming back, a copy that failed, the outcome of a form. Each message is a full sentence that stands alone.
- Nothing else sets `role="status"`, `role="alert"` or `aria-live`: not a row, not a field's help, not the status card or the activity feed. What is worth seeing is on the page as plain text.
- `ErrorBoundary` is the one other place that speaks for the page: when a page cannot be drawn it shows a card with Reload and the actions the app gives it, such as Sign out.
