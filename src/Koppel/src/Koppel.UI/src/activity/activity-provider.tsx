import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback, useMemo, useRef, useState, type ReactNode } from 'react';
import { getActivity, type ServerActivity } from '@/api/activity';
import { useHubEvent } from '@/hub/hub-context';
import { ActivityContext, fromServer, type ActivityItem } from './activity-context';

const activityKey = ['activity'] as const;
const maxItems = 200;

/**
 * The activity log: the server's log (loaded once, then followed over the hub, so it survives a
 * reload and matches on every device), plus our own changes that the server has not answered yet.
 */
export function ActivityProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const server = useQuery({ queryKey: activityKey, queryFn: getActivity });
  const [local, setLocal] = useState<ActivityItem[]>([]);
  const next = useRef(0);

  const onRecorded = useCallback(
    (event: ServerActivity) =>
      queryClient.setQueryData<ServerActivity[]>(activityKey, (current) => [event, ...(current ?? []).filter((e) => e.id !== event.id)].slice(0, maxItems)),
    [queryClient],
  );
  useHubEvent('ActivityRecorded', onRecorded);

  const add = useCallback((item: Omit<ActivityItem, 'id' | 'at'>) => {
    next.current += 1;
    const id = `local-${next.current}`;
    setLocal((current) => [{ ...item, id, at: new Date() }, ...current].slice(0, maxItems));
    return id;
  }, []);

  const update = useCallback((id: string, patch: Partial<Omit<ActivityItem, 'id'>>) => {
    setLocal((current) => current.map((item) => (item.id === id ? { ...item, ...patch, at: new Date() } : item)));
  }, []);

  const remove = useCallback((id: string) => setLocal((current) => current.filter((item) => item.id !== id)), []);

  const items = useMemo(
    () => [...local, ...(server.data ?? []).map(fromServer)].sort((a, b) => b.at.getTime() - a.at.getTime()),
    [local, server.data],
  );

  const value = useMemo(() => ({ items, add, update, remove }), [items, add, update, remove]);
  return <ActivityContext.Provider value={value}>{children}</ActivityContext.Provider>;
}
