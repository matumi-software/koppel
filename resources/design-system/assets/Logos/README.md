# Logos

The Koppel mark: a K whose arms end in a circle and an arrow, after the USB trident. The circle is also the Shared state glyph.

- `koppel-mark.svg`: single ink, `accent` light value `#2A6B4E`. Use on `bg-page`, `bg-surface` and other light grounds.
- `koppel-mark-on-dark.svg`: single ink, `accent` dark value `#549572`. Use on dark grounds.
- `koppel-app-icon.svg`: the app icon, the mark in `bg-page` light `#FBF9F4` on an `accent` light `#2A6B4E` tile with 14/64 corner radius. The tile keeps it legible on light and dark Windows themes. Use it wherever the system shows Koppel as an app: the Apps list, setup.exe, Koppel.exe.
- `koppel-app-icon.ico` and `koppel-app-icon-64.png` are rendered from it by `pnpm design:icons` (run from `src/`). Do not edit them by hand.
- Clear space: half the mark's width on every side. Minimum size: 16px.
- The wordmark is set in type ("Koppel", Instrument Sans 600) beside the mark. No outlined wordmark file exists yet.
