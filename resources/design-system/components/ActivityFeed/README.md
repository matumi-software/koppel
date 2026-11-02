# ActivityFeed

Every change in this session as the server confirms or rejects it, newest first.

- `kp-feed` list with `role="log"`, items `kp-feed__item` with `data-state` and a `kp-glyph` in the state's shape.
- Lead words are fixed: **Waiting.** while pending, **Done.** when confirmed, **Not done.** with the reason when rejected or timed out.
- It sits in the aside when no device is selected, and filtered to one device as that device's History in the detail drawer.
