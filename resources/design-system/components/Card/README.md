# Card, Stack and Row

The containers a page is built from. A page never sets its own flex, gap or padding.

- `Card` (`kp-card`): `bg-surface` with a 1px `border-default` edge, `radius-lg`, 16px padding and a 16px gap. It is a labelled region (`aria-label`). `data-gap="sm"` (8px) and `data-gap="md"` (12px) tighten it; `data-flush` removes the padding and gap for a card that holds a list of rows that draw their own.
- `Stack` (`kp-stack`): children one under the other, 16px apart by default; `data-gap` is `sm` 8px, `md` 12px or `lg` 24px. `data-align="start"` keeps children at their own width.
- `Row` (`kp-row`): children side by side, 8px apart, wrapping when the room runs out. `data-gap="xs"` is 4px. `data-justify` is `end` for a row of buttons at the right of a dialog or form, or `between`.
- Text inside them: `Note` (`kp-note`, `kp-error` for what went wrong, `kp-ok` for what went right), `Caption` (`kp-type-caption`, `data-tone` is `faint`, `pending` or `fault`) and `SectionHeading` (`kp-type-heading` without margins). None of them is a live region: the Announcer says what a screen reader needs to hear.
