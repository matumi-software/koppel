import { useCallback, useMemo, useRef, useState, type ReactNode } from 'react';
import { ActivityContext, type ActivityItem } from './activity-context';

const maxItems = 100;

/** Keeps this session's activity: every change sent, and what the server said about it. */
export function ActivityProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ActivityItem[]>([]);
  const next = useRef(0);

  const add = useCallback((item: Omit<ActivityItem, 'id' | 'at'>) => {
    next.current += 1;
    const id = `a${next.current}`;
    setItems((current) => [{ ...item, id, at: new Date() }, ...current].slice(0, maxItems));
    return id;
  }, []);

  const update = useCallback((id: string, patch: Partial<Omit<ActivityItem, 'id'>>) => {
    setItems((current) => current.map((item) => (item.id === id ? { ...item, ...patch, at: new Date() } : item)));
  }, []);

  const value = useMemo(() => ({ items, add, update }), [items, add, update]);
  return <ActivityContext.Provider value={value}>{children}</ActivityContext.Provider>;
}
