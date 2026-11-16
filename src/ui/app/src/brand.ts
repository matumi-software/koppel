import type { Brand, StateMap } from '@matumi/ui';

/**
 * Koppel's fixed state words: what a StateLabel or StatusDot shows for a device, a remote device or a
 * link. Attached is Koppel's own tone (`--state-attached`, koppel.css) with the hexagon glyph. The
 * neutral words are drawn the same way on every screen.
 */
export const states: StateMap = {
  Shared: { tone: 'success' },
  Attached: { tone: 'attached', shape: 'hexagon' },
  Pending: { tone: 'pending' },
  Offline: { tone: 'danger' },
  'Not shared': { tone: 'neutral' },
  'Not attached': { tone: 'neutral' },
  'In use': { tone: 'neutral' },
};

/** The K mark, as Heartwood's BrandSlot carries it, with Koppel's states. */
export const brand: Brand = {
  name: 'koppel',
  display: 'Koppel',
  states,
  mark: '<g fill="currentColor" stroke="currentColor"><rect x="9" y="8" width="11" height="48" rx="2" stroke="none"/><path d="M20 32 L41 15" stroke-width="9" fill="none"/><path d="M20 32 L41.7 47.8" stroke-width="9" fill="none"/><path d="M53 56 L35.6 54.5 L46.2 39.9 Z" stroke-width="3" stroke-linejoin="round"/></g><circle cx="46" cy="12" r="8" class="canopy-fill"/>',
};
