# Logos

The Koppel mark: a K whose arms end in a circle and an arrow, after the USB trident. The circle is also the Shared state glyph.

- `koppel-mark.svg`: the K in `text-primary` light `#1C1B18`, the circle in `accent` light `#2A6B4E`. Use on `bg-page`, `bg-surface` and other light grounds.
- `koppel-mark-on-dark.svg`: the K in `text-primary` dark `#EDEAE0`, the circle in `accent` dark `#549572`. Use on dark grounds.
- Since the 2026 Matumi rebrand every family mark has exactly one canopy element; Koppel's is the circle.
- `src/ui/app/public/favicon.svg`: the browser tab icon, the mark in both sets of colours, switched by `prefers-color-scheme`. Keep it in step with the two files above.
- `koppel-app-icon.svg`: the app icon, the mark in `bg-page` light `#FBF9F4` on an `accent` light `#2A6B4E` tile with 14/64 corner radius. The tile keeps it legible on light and dark Windows themes. Use it wherever the system shows Koppel as an app: the Apps list, setup.exe, Koppel.exe, the Linux applications menu.
- `koppel-app-icon.ico`, `koppel-app-icon-64.png`, `koppel-app-icon-1000.png` and the Linux package's icons in `src/installer/linux/icons` are rendered from it by `pnpm design:icons` (run from `src/`). Do not edit them by hand.
- `koppel-app-icon-1000.png` is the Docker Hub repository logo for `matumisoftware/koppel`, at Docker Hub's largest size. It is uploaded by hand on the repository page.
- Clear space: half the mark's width on every side. Minimum size: 16px.
- The wordmark is set in type ("koppel", all lowercase, Instrument Sans 600) beside the mark. No outlined wordmark file exists yet.
