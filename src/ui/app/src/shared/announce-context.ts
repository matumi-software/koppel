import { createContext, useContext } from 'react';

export type Announce = (message: string) => void;

export const AnnouncerContext = createContext<Announce>(() => {});

/**
 * Says something to a screen reader: "Saved", "Not saved: the device is in use". Without an
 * AnnouncerProvider above it, it does nothing, so a component works on its own in a test.
 */
export const useAnnounce = (): Announce => useContext(AnnouncerContext);
