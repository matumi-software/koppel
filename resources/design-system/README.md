Koppel shares USB devices across the network: a server offers the devices plugged into it, and a client attaches them as if they were local. *Koppel* is Afrikaans and Dutch for "couple" or "link". Koppel is a Matumi product and shares Matumi's colours: **canopy** green on warm **stone** neutrals, with Matumi's **river** for devices in use. It should feel like a well-labelled patch panel: precise, quiet and trustworthy, with state you can read from across the room.

## Product rule: the server confirms, the UI never assumes

Every change goes to the server over REST and is only shown as done when the server pushes the confirmed state back over SignalR, after it has written the change to its config file. The design system makes that wait visible everywhere.

- A control the user just changed moves to **pending**: the stripe pattern in `pending` over `pending-subtle`, plus a word ("Saving…"). It never jumps straight to its new confirmed look.
- When the confirmation arrives, the control settles into its confirmed state and the caption reads "Saved". When the server rejects or times out, the control returns to its last confirmed state, turns `fault` and says why ("Not saved: device is in use").
- When the live link drops, show `LinkBanner` in its reconnecting or down state and disable every control that would send a change. Nothing can be confirmed without the link, so nothing may look editable.
- The stripe means "waiting for the server" and nothing else. Never use it for decoration, progress bars or loading skeletons.

## Content fundamentals

- Write from the user's side of the cable: "Share", "Attach", "Detach", "Stop sharing". Not "export", "bind", "import" or "URB".
- Sentence case everywhere: buttons, tabs, titles. "Attach device", not "Attach Device".
- Address the user as "you". The product speaks in plain statements, no exclamation marks, no emoji.
- Show device identity the way the hardware reports it: vendor and product name first, then `VID:PID` and the bus ID in `mono` (`0403:6001`, `bus 1-1.4`). Hosts appear as the name people gave them, with `host:port` in `mono` next to it when it matters.
- State words are fixed: **Shared**, **Attached**, **Pending**, **Offline**, **Not shared**. Use them in badges, filters and logs exactly as written.
- Errors say what happened and what to do: "Not saved: the server rejected the change because the device is attached to bench-2. Detach it there first."
- Examples of real copy: "Plug a device into this machine to share it.", "Attached to lab-rack-3 since 09:14", "Reconnecting. Changes are paused until the link is back."

## Color

- Page ground is `bg-page`. Cards, device rows and panels are `bg-surface`. Recessed wells (inputs, ID chips) are `bg-inset`. Menus and dialogs are `bg-raised` with `shadow-raised`.
- Text is `text-primary`, supporting text `text-secondary`, captions and placeholders `text-tertiary`. All three pass 4.5:1 on `bg-page`, `bg-surface` and `bg-inset` in both themes.
- `accent` (canopy green) is the only accent. Use it for the primary action, the selected tab or segment, links and the mark, and at most one accent button per view. Text on an accent fill is `on-accent`.
- Device state has its own colors, separate from the accent: `shared` Shared (canopy, Matumi's healthy colour), `attached` Attached (river), `pending` Pending, `fault` Offline. Each has a `-subtle` background and a `-text` foreground for badges.
- State is never told by color alone. Every state carries its word and its glyph: a circle for Shared, a square for Attached, a stripe for Pending, a cross for Offline and an empty ring for Not shared. The circle and square are the two terminals of the mark.
- Dark is the primary theme; light is a full equal. Check both before shipping a screen.

## Type

- Interface text is set in Instrument Sans (`--font-sans`), loaded from Google Fonts with a Segoe UI and system-ui fallback.
- Identifiers are set in JetBrains Mono (`--font-mono`): VID:PID, bus IDs, ports, client IDs, file paths. Use `font-variant-numeric: tabular-nums` wherever IDs line up in columns.
- Use the scale as written: `display` for the one page title, `title` for sections and dialogs, `heading` for device names, `body` for default text, `label` for buttons, tabs, labels and badges, `caption` for timestamps and help, `mono` and `mono-sm` for identifiers.

## Space, shape and depth

- Spacing runs on a 4px base: `space-1` 4, `space-2` 8, `space-3` 12, `space-4` 16, `space-6` 24, `space-8` 32. Card padding is `space-4`; the gap between cards is `space-6`.
- Radii: `radius-sm` for badges and ID chips, `radius-md` for buttons and inputs, `radius-lg` for cards and dialogs, `radius-pill` only for the switch and the live-link dot.
- Separate with borders, not shadows. Cards take a 1px `border-default` edge; controls take `border-control`, which holds 3:1. `shadow-raised` is only for things floating above the page.

## Focus and motion

- Keyboard focus is a 2px solid `focus-ring` with a 2px offset, on every interactive element. It holds 3:1 on every surface.
- Motion is functional only: the moving stripe while pending, the switch thumb sliding (120ms), the live dot breathing. All three stop under `prefers-reduced-motion`.

## Iconography

- The mark is a K whose arms end in a circle and a square, the shapes used on the USB trident's ends. Use `assets/Logos/koppel-mark.svg` on light grounds and `koppel-mark-on-dark.svg` on dark grounds. Keep clear space of half the mark's width around it and never draw it below 16px.
- The wordmark is "Koppel" set in Instrument Sans 600 next to the mark, with the mark's height matched to the cap height plus ascender. There is no outlined wordmark file yet; set it in type.
- UI icons: use Lucide (outline, 1.5px stroke, 16px in controls and 20px in headings), drawn in `currentColor`. No emoji anywhere in the product.

## Components

Components here are the reference look, written as CSS classes with a `kp-` prefix and state in `data-state`. The React implementations live in the repo's own UI library on top of react-aria-components and use these exact class names and tokens.

- `Button`: primary, secondary, ghost and danger, with a sending state while the REST call is in flight.
- `Switch`: on, off, pending-on, pending-off and failed. The core of the confirm-on-push rule.
- `StatusBadge`: the five device states.
- `DeviceCard`: one USB device with its identity, where it is and the action for this role.
- `LinkBanner`: the live link to the server: live, reconnecting or down.
- `Field`: text, ID and secret inputs for auth and settings, with pending and invalid states.
- `SegmentedControl`: switches the view between the Server and Client roles.
