# Koppel brand assets

Koppel shares USB devices across the network: a server offers the devices plugged into it, and a client attaches them as if they were local. *Koppel* is Afrikaans and Dutch for "couple" or "link". Koppel is a Matumi product and shares Matumi's colours: **canopy** green on warm **stone** neutrals, with Matumi's **river** for devices in use. It should feel like a well-labelled patch panel: precise, quiet and trustworthy, with state you can read from across the room.

What is here: the logos and the app icon in `assets/Logos` (see its README), the social images in `assets/Social`, and below, what Koppel says and how it behaves, which is Koppel's own. Colours, type, spacing, focus, motion, icons and every component come from Heartwood, the Matumi design system (`@matumi/tokens` and `@matumi/ui`); how the UI uses them is in `docs/ui.md`.

## Product rule: the server confirms, the UI never assumes

Every change goes to the server over REST and is only shown as done when the server pushes the confirmed state back over Server-Sent Events, after it has written the change to its config file. The UI makes that wait visible everywhere.

- A control the person just changed shows that it is waiting, with a word, and never jumps straight to its new confirmed look.
- When the confirmation arrives, the control settles into its confirmed state. When the server rejects the change or does not answer in time, the control goes back to its last confirmed state and says why ("Not saved: device is in use").
- When the live link drops, the UI says so once and disables every control that would send a change. Nothing can be confirmed without the link, so nothing may look editable.

## Content fundamentals

- Write from the user's side of the cable: "Share", "Attach", "Detach", "Stop sharing". Not "export", "bind", "import" or "URB".
- Sentence case everywhere: buttons, tabs, titles. "Attach device", not "Attach Device".
- Address the user as "you". The product speaks in plain statements, no exclamation marks, no emoji.
- Show device identity the way the hardware reports it: vendor and product name first, then `VID:PID` and the bus ID in `mono` (`0403:6001`, `bus 1-1.4`). Hosts appear as the name people gave them, with `host:port` in `mono` next to it when it matters.
- State words are fixed: **Shared**, **Attached**, **Pending**, **Offline**, **Not shared**. Use them in badges, filters and logs exactly as written.
- Errors say what happened and what to do: "Not saved: the server rejected the change because the device is attached to bench-2. Detach it there first."
- Examples of real copy: "Plug a device into this machine to share it.", "Attached to lab-rack-3 since 09:14", "Reconnecting. Changes are paused until the link is back."
