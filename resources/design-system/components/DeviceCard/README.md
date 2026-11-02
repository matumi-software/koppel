# DeviceCard

Shows one USB device: what it is, where it is plugged in, its state and the action for this role.

- Class `kp-device`, with `kp-device__head` (the `kp-device__name` heading and a `StatusBadge`), `kp-device__ids` (`kp-id` chips for VID:PID and bus ID), `kp-device__where` (host and since when) and `kp-device__foot` (the action).
- Name: the product string the device reports, or the nickname the user gave it. Long names wrap; they never truncate the badge.
- IDs are always `mono`: `0403:6001`, `bus 1-1.4`.
- On the Server view the foot holds a `Switch` labelled "Share this device". On the Client view it holds a Button: "Attach" (primary when this is the device the user filtered for, else secondary) or "Detach" (secondary). Use "Force detach" (danger) only for an admin taking a device from another client.
- When any change to the device is pending, the badge shows Pending and the action is held in its pending or sending state until SignalR confirms.
- `data-selected="true"` marks the card the user opened in the detail panel.
- Lay cards out in a grid with `space-6` gaps, one column under 640px.
- The consumer provides the device record (name, VID, PID, bus ID, host, state, since), the role and the action handler.
