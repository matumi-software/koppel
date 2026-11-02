# Setup

The first run: pick the role and see whether this machine can do it.

- `kp-steps` shows where the user is. `kp-rolecards` is a react-aria RadioGroup of `kp-rolecard` Radios (Server, Client, Both).
- `kp-checks` lists what the chosen role needs: `data-state="ok"` with a tick, or `fault` with a cross and the exact command in `kp-cmd`.
- Continue saves the role and marks setup done, and waits for the server like any other change. A failed check warns but does not block.
