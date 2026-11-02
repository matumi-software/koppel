# AppShell

The app frame: sidebar navigation, the page, and an aside for activity or a device's details.

- Classes: `kp-shell`, `kp-shell__side`, `kp-shell__main`, `kp-shell__aside` (`data-wide="true"` for details), `kp-shell__top` (phone only), `kp-shell__foot` (link status at the bottom of the sidebar, `data-state` live, reconnecting or down).
- Navigation is `kp-nav` groups of `kp-nav__item` react-aria Links, with `aria-current="page"` on the open page and an optional `kp-nav__count` (`data-tone="pending"` when it counts things that need you).
- Groups: Overview; Sharing from <machine> (Devices, Clients) for servers; Attaching (From <server>) for clients; Settings (This machine, Server connection).
- At 760px and below the sidebar moves into a menu (react-aria ModalOverlay + Modal) behind a top-bar button, and the aside folds below the page. Nothing may be wider than the screen: no sideways scrolling on a phone.
- The open page lives in the address (`#devices`), so a phone can bookmark the page it checks.
