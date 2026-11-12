import { useEffect, useState } from 'react';

/**
 * The current time in milliseconds, renewed every `everyMs`, so text such as "seen 3 min ago" moves
 * on without something else having to re-render the page. Take the time from here, never from
 * `Date.now()` in a render.
 */
export function useNow(everyMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(id);
  }, [everyMs]);
  return now;
}
