Koppel shares USB devices across the network: a server offers the devices plugged into it, and a client attaches them as if they were local. *Koppel* is Afrikaans and Dutch for "couple" or "link". Koppel is a Matumi product and shares Matumi's colours: **canopy** green on warm **stone** neutrals, with Matumi's **river** for devices in use. It should feel like a well-labelled patch panel: precise, quiet and trustworthy, with state you can read from across the room.

## Product rule: the server confirms, the UI never assumes

Every change goes to the server over REST and is only shown as done when the server pushes the confirmed state back over SignalR, after it has written the change to its config file. The design system makes that wait visible everywhere.

- A control the user just changed moves to **pending**: the stripe pattern in `pending` over `pending-subtle`, plus a word ("Saving…"). It never jumps straight to its new confirmed look.
- When the confirmation arrives, the control settles into its confirmed state and the caption reads "Saved". When the server rejects or times out, the control returns to its last confirmed state, turns `fault` and says why ("Not saved: device is in use").
- When the live link drops, show the link as reconnecting or down in `StatusCard` (and say so once, through the Announcer) and disable every control that would send a change. Nothing can be confirmed without the link, so nothing may look editable.
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
- `accent` (canopy green) is the only accent. Use it for the primary action, the selected tab or segment, links and the mark's circle, and at most one accent button per view. Text on an accent fill is `on-accent`.
- Device state has its own colors, separate from the accent: `shared` Shared (canopy, Matumi's healthy colour), `attached` Attached (river), `pending` Pending, `fault` Offline. Each has a `-subtle` background and a `-text` foreground for badges.
- State is never told by color alone. Every state carries its word and its glyph: a circle for Shared, a square for Attached, a stripe for Pending, a cross for Offline and an empty ring for Not shared. The circle is the mark's terminal and the square comes from the USB trident.
- Dark is the primary theme; light is a full equal. Check both before shipping a screen.

## Type

- Interface text is set in Instrument Sans (`--font-sans`), self-hosted by the UI library (Latin and Latin Extended, woff2 only), with a Segoe UI and system-ui fallback that also draws the scripts it does not carry.
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

- The mark is a K whose arms end in a circle and an arrow, two of the shapes on the USB trident. Use `assets/Logos/koppel-mark.svg` on light grounds and `koppel-mark-on-dark.svg` on dark grounds. Keep clear space of half the mark's width around it and never draw it below 16px.
- The wordmark is "koppel", always lowercase like the other Matumi Software wordmarks, set in Instrument Sans 600 next to the mark, with the mark's height matched to the cap height plus ascender. There is no outlined wordmark file yet; set it in type.
- The social preview for GitHub and link cards is `assets/Social/koppel-social-preview.png` (1280x640). Its source is `koppel-social-preview.html` next to it: open it in a browser at 1280x640 and screenshot the card to regenerate it.
- UI icons: use Lucide (outline, 1.5px stroke, 16px in controls and 20px in headings), drawn in `currentColor`. No emoji anywhere in the product.

## Layout

- The app is a shell: sidebar navigation, the page, and an aside. The aside is one panel at a width the user sets: it shows Activity, or the selected device's or client's details, and what it shows never changes its width.
- Settings pages use Panels with settings rows, like the Overview, and the role uses the same role cards as the first run.
- The Overview leads with Needs you when anything is pending or rejected, then what this machine shares and what it has attached.
- Devices are grouped by what they need (Waiting for the server, In use, Shared and free, Not shared or offline), not listed alphabetically.
- At phone width the sidebar becomes a menu and the aside folds below the page. Nothing scrolls sideways. A phone is for checking up on devices, so the Overview and device pages must read well there.

## Components

Components here are the reference look, written as CSS classes with a `kp-` prefix and state in `data-state`. The React implementations live in the repo's own UI library on top of react-aria-components and use these exact class names and tokens.

- `Button`: primary, secondary, ghost and danger, with a sending state while the REST call is in flight.
- `Switch`: on, off, pending-on, pending-off and failed. The core of the confirm-on-push rule.
- `StatusBadge`: the five device states.
- `DeviceCard`: one USB device with its identity, where it is and the action for this role.
- `StatusCard`: the foot of the sidebar: the live link, this machine and the Koppel version it runs, what a client is linked to, and Sign out.
- `Field`: text, ID and secret inputs for auth and settings, with pending and invalid states.
- `Card`, `Stack`, `Row`: the containers pages are built from, so no page sets its own flex, gap or padding.
- `Form`, `FieldGroup`, `LabeledValue`: a form with react-aria validation, fields that belong together, and a value that is not an input, with a label like a field's.
- `Drawer`: the selected device's or client's details in the aside, with its sections.
- `FirstRunShell`: the plain page that sign-in and first run sit on.
- `Announcer`: the one live region.
- `ErrorBoundary`: what a page that cannot be drawn shows in its place.
- `AppShell`, `DeviceGroup`, `ActivityFeed`, `DeviceDrawer`, `Panel`, `Setup`, `ModalDialog`: the app frame and pages, built on react-aria-components.
