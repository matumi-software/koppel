import { useQuery } from '@tanstack/react-query';
import { getSettings } from '@/api/settings';
import { useHub } from '@/hub/hub-context';
import { ClientDevicesView } from './client-devices';
import { ServerDevicesView } from './server-devices';

/** The devices this machine shares, attaches, or both, depending on its confirmed role. */
export function DevicesPage() {
  const { state: link } = useHub();
  const settings = useQuery({ queryKey: ['settings'], queryFn: getSettings });
  const locked = link !== 'live';

  if (!settings.data) {
    return <p className="app-note">Loading…</p>;
  }

  const role = settings.data.role;
  return (
    <div className="app-stack app-stack--wide">
      {role !== 'client' && <ServerDevicesView locked={locked} />}
      {role !== 'server' && <ClientDevicesView locked={locked} />}
    </div>
  );
}
