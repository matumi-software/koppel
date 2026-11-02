# SegmentedControl

Switches between two or three views of the same thing, such as the Server and Client roles.

- Class `kp-segment` holding `kp-segment__item` buttons; the selected one has `aria-checked="true"` and takes an `accent` edge and `accent-text`.
- Use it for the role switch in the app header ("Server", "Client") when the machine runs both roles, and for small view filters ("All", "Shared", "Attached"). For more than three options use tabs or a select.
- It changes what the user sees, not what the server does, so it switches at once with no pending state.
- In React it is a react-aria `RadioGroup` rendered as segments; the consumer provides the options, the selected value and the change handler.
