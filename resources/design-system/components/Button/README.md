# Button

Starts an action: share, attach, detach, save settings.

- Class `kp-button` with `data-variant` set to `primary`, `secondary`, `ghost` or `danger`.
- `primary` is the canopy `accent` and appears at most once per view, on the action the view exists for ("Attach device").
- `secondary` for the other actions, `ghost` for low-weight actions inside cards and toolbars, `danger` for actions that cut a live connection ("Force detach", "Revoke client").
- Set `data-state="sending"` while the REST call is in flight. The button shows a spinner and keeps its label. A finished REST call does not mean the change happened: the control the button affects shows pending until the server confirms over SignalR.
- Disable every button that sends a change while the live link is not live (`StatusCard` shows it).
- Labels are a verb plus an object in sentence case: "Stop sharing", "Attach device". No "OK", no "Submit".
- The consumer provides the label, the variant and the click handler. In React it is a react-aria `Button`, so `onPress`, `isDisabled` and keyboard handling come from there.
