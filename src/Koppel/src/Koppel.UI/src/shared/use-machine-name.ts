import { useQuery } from '@tanstack/react-query';
import { getIdentity } from '@/api/server';

/** This machine's name as the server reports it; the browser's host name until that loads. */
export function useMachineName(): string {
  const identity = useQuery({ queryKey: ['identity'], queryFn: getIdentity, staleTime: Infinity });
  return identity.data?.name ?? window.location.hostname;
}
