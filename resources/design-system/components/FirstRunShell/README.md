# FirstRunShell and PageHead

The plain page that sign-in, the first password and the first run sit on: the app shell would show navigation to a browser that is not signed in, so these pages have their own.

- `kp-firstrun`: a bar with the mark and the wordmark "koppel" on `bg-surface`, then one centred column, 460px wide (`data-width="wide"`, 920px, for the role step).
- It sets the browser tab's title to the page's name and the product.
- `PageHead` (`kp-page-head`) is the page's h1 and the sentence under it; the app's pages use it too. A page that opens because the person went to it moves focus to its h1 so a keyboard or a screen reader starts at the new page.
