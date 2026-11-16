import { useEffect, useState } from 'react';

/** True while the media query matches, and follows it. */
export function useMedia(query: string): boolean {
  const [matches, setMatches] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const list = window.matchMedia(query);
    const on = () => setMatches(list.matches);
    on();
    list.addEventListener('change', on);
    return () => list.removeEventListener('change', on);
  }, [query]);
  return matches;
}
