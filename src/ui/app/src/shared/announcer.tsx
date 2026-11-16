import { useCallback, useMemo, useState, type ReactNode } from 'react';
import { AnnouncerContext } from './announce-context';

/**
 * The one live region of the app. Everything the app wants announced goes through it, so a screen
 * reader hears one calm voice instead of one region per row. Nothing else sets role="status",
 * role="alert" or aria-live. Messages stay out of sight (`.kp-announcer`, koppel.css); what is worth
 * seeing is on the page, or in a toast.
 */
export function AnnouncerProvider({ children }: { children: ReactNode }) {
  // The same sentence twice in a row still has to be heard twice, so each is a new entry.
  const [entry, setEntry] = useState({ id: 0, message: '' });
  const announce = useCallback((message: string) => setEntry((current) => ({ id: current.id + 1, message })), []);
  const value = useMemo(() => announce, [announce]);
  return (
    <AnnouncerContext.Provider value={value}>
      {children}
      <div className="kp-announcer" role="status" aria-live="polite" aria-atomic="true">
        <span key={entry.id}>{entry.message}</span>
      </div>
    </AnnouncerContext.Provider>
  );
}
