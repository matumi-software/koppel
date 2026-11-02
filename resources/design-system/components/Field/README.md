# Field

A labelled text input for settings and authentication: server address, client ID, client secret.

- Class `kp-field` with `kp-field__label`, a `kp-field__control` row holding a `kp-input` (and an optional ghost Button), and `kp-field__help`.
- Set `data-mono="true"` on the input for identifiers: client IDs, `host:port`, paths.
- Secrets: the input is `type="password"` with a "Show" ghost button. The server never sends a stored secret back; show the placeholder "Stored, enter a new one to replace it" and the help "Encrypted on disk".
- `data-state` on the field: none, `pending` (saved, waiting for the server to confirm the config write; help reads "Saving…") or `invalid` (help says what to fix).
- Settings save per section with a Button, not on every keystroke. A section's fields go pending together and settle together on the confirmation.
- In React it is a react-aria `TextField`; the consumer provides the label, value, help text, state and change handler.
