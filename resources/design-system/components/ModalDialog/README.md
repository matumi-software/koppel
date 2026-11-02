# ModalDialog

A react-aria modal dialog, used for the one-time client secret.

- `kp-overlay` with `data-placement="center"` around a `kp-dialog`. Focus moves in and stays, Escape closes it, a Button with `slot="close"` closes it too.
- Use it only for something that must be read before moving on. Everything else stays on the page.
