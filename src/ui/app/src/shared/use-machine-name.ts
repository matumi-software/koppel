import { useQuery } from '@tanstack/react-query';
import { systemInfoOptions } from '@/queries';

/** The system info the side panel shows. One query, shared by the machine name and the version. */
const useSystemInfo = () => useQuery(systemInfoOptions());

/** This machine's name as the server reports it; the browser's host name until that loads. */
export function useMachineName(): string {
  return useSystemInfo().data?.name ?? window.location.hostname;
}

/** The Koppel version the server runs, such as 0.1.1, without a commit hash. Undefined until it loads. */
export function useKoppelVersion(): string | undefined {
  return useSystemInfo().data?.version;
}

/** True when this machine has a tray icon (the Windows app), so it can show notifications. False until it loads. */
export function useHasTray(): boolean {
  return useSystemInfo().data?.tray ?? false;
}
