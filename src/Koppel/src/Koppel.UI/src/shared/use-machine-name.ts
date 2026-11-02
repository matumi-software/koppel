import { useQuery } from '@tanstack/react-query';
import { getSystemInfo } from '@/api/http';

/** This machine's name as the server reports it; the browser's host name until that loads. */
export function useMachineName(): string {
  const info = useQuery({ queryKey: ['system-info'], queryFn: getSystemInfo, staleTime: Infinity });
  return info.data?.name ?? window.location.hostname;
}
