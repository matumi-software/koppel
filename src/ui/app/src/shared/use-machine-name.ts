import { useQuery } from '@tanstack/react-query';
import { systemInfoOptions } from '@/queries';

/** This machine's name as the server reports it; the browser's host name until that loads. */
export function useMachineName(): string {
  return useQuery(systemInfoOptions()).data?.name ?? window.location.hostname;
}
