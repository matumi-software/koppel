# Switch

Turns a setting on or off, and shows that the change is pending until the server confirms it.

- Class `kp-switch` holding `kp-switch__track` (with `kp-switch__thumb`), the label, and an optional `kp-switch__note` caption.
- `data-state` is one of `off`, `on`, `pending-on`, `pending-off` or `failed`.
- The flow: the user flips the switch, the UI sends the REST call and moves to `pending-on` or `pending-off`. The thumb shows where the user asked to go; the stripe says the server has not confirmed it yet, and the note reads "Saving…". When SignalR pushes the confirmed state, the switch moves to `on` or `off` and the note reads "Saved". When the server rejects the change or no confirmation arrives in time, the switch returns to its last confirmed position with `data-state="failed"`, and the note says why.
- Never set `on` or `off` from the click itself. Only a confirmed state from the server may do that.
- In a settings row, which already names it, the label is for screen readers only (`labelHidden`).
- While pending, ignore further clicks on the same switch. While `LinkBanner` is not live, set `aria-disabled="true"`.
- The consumer provides the label, the confirmed value, the pending target (if any), the failure reason (if any) and the change handler. In React it is a react-aria `Switch`; `isSelected` follows the pending target while pending so screen readers announce the requested value, and the note is linked with `aria-describedby`.
