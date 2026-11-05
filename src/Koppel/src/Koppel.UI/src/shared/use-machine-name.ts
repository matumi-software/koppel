import { useQuery } from '@tanstack/react-query';
import { getSystemInfo } from '@/api/http';
import { systemInfoKey } from '@/query-keys';

/** The system info the side panel shows. One query, shared by the machine name and the version. */
const useSystemInfo = () => useQuery({ queryKey: systemInfoKey, queryFn: getSystemInfo, staleTime: Infinity });

/** This machine's name as the server reports it; the browser's host name until that loads. */
export function useMachineName(): string {
  return useSystemInfo().data?.name ?? window.location.hostname;
}

/** The Koppel version the server runs, such as 0.1.1, without a commit hash. Undefined until it loads. */
export function useKoppelVersion(): string | undefined {
  return useSystemInfo().data?.version;
}
