# AppShell

The app frame: sidebar navigation, the page, and an aside for activity or a device's details.

- Classes: `kp-shell`, `kp-shell__side`, `kp-shell__main`, `kp-shell__aside` holding `kp-shell__resize` and `kp-shell__aside-body`, `kp-shell__top` (phone only), `kp-shell__foot` (holds `StatusCard` at the bottom of the sidebar and the phone menu; the card draws its own ground and state).
- The aside is one panel for every page. Pages change what it shows (facts and Activity, or the selected device's or client's details), never its width, so selecting a card never resizes the page or the cards on it.
- Its width is the user's: `--kp-aside-width`, 340px to start, 280 to 560px, at most 45% of the window. `kp-shell__resize` on its left edge is a focusable `separator`: drag it, or use the arrow keys (16px steps) and Home or End. The app keeps the width the user left it at.
- Navigation is `kp-nav` groups of `kp-nav__item` react-aria Links, with `aria-current="page"` on the open page and an optional `kp-nav__count` (`data-tone="pending"` when it counts things that need you).
- Groups: Overview; Sharing from <machine> (Devices, Clients) for servers; Attaching (From <server>) for clients; Settings (This machine, Server connection).
- At 760px and below the sidebar moves into a menu (react-aria ModalOverlay + Modal) behind a top-bar button, and the aside folds below the page at full width, without the resize edge. Nothing may be wider than the screen: no sideways scrolling on a phone.
- The open page lives in the address (`#devices`), so a phone can bookmark the page it checks.
