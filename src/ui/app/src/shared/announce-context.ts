import { announce } from '@heartwood/ui';

export type Announce = (message: string) => void;

/**
 * Says something to a screen reader: "Saved", "Not saved: the device is in use". It speaks through
 * Heartwood's one live region (the ToastRegion's), which a toast also uses, so a screen reader hears one
 * calm voice. A toast announces itself: do not announce the same event again beside it.
 */
export const useAnnounce = (): Announce => announce;
