import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import { ActivityPage } from '@/activity/activity-page';
import { ClientsPage } from '@/clients/clients-page';
import { ConnectionPage } from '@/connection/connection-page';
import { DevicesPage } from '@/devices/devices-page';
import { RemoteDevicesPage } from '@/devices/remote-devices-page';
import { useLive } from '@/live/live-context';
import { OverviewPage } from '@/overview/overview-page';
import { useNeedsYou } from '@/overview/use-needs-you';
import { setupKey } from '@/pending/domains';
import { usePending } from '@/pending/pending-context';
import { settingsOptions } from '@/queries';
import { SecurityPage } from '@/security/security-page';
import { SettingsPage } from '@/settings/settings-page';
import { SetupPage } from '@/setup/setup-page';
import { CouldNotReach } from '@/shell/could-not-reach';
import { usePage } from '@/shell/frame-context';
import { KoppelShell } from '@/shell/koppel-shell';
import { Loading } from '@/shell/loading';
import { pagesOf, type Page } from '@/shell/places';
import { SignedOutFrame } from '@/shell/signed-out-frame';
import { CheckPage } from '@/system/check-page';

const pages: Record<Page, () => React.JSX.Element> = {
  overview: OverviewPage,
  devices: DevicesPage,
  remote: RemoteDevicesPage,
  clients: ClientsPage,
  activity: ActivityPage,
  check: CheckPage,
  connection: ConnectionPage,
  security: SecurityPage,
  settings: SettingsPage,
};

/**
 * What the signed-in app shows: the first-run role step until setup is confirmed, then the shell
 * with the page the address names. The live link is the shell's concern (`locked`), not a reason to
 * hide a page: what loaded stays on screen while it reconnects.
 */
export function App() {
  const { retry } = useLive();
  const { state: link } = useLive();
  const locked = link !== 'live';

  const settings = useQuery(settingsOptions());
  // First run lands on the page the new role needs first, once the server has confirmed setup. After
  // that, an address with no page is the overview.
  const { completed } = usePending('settings');
  const justSetUp = (completed[setupKey] ?? 0) > 0;
  const role = settings.data?.role;
  const page = usePage(justSetUp ? (role === 'client' ? 'connection' : 'clients') : 'overview');

  // A page that does not exist for this role falls back to the overview, once the role is known:
  // before that a bookmarked #remote would be thrown away on every load.
  const shown: Page = !settings.data || pagesOf(role).includes(page) ? page : 'overview';

  // The nav badge on Overview counts the same list Overview shows as Needs you.
  const needs = useNeedsYou(locked).length;
  const counts = useMemo(() => ({ overview: needs || undefined }), [needs]);

  if (!settings.data) {
    return settings.isError ? (
      <SignedOutFrame flush>
        <CouldNotReach isRetrying={settings.isFetching} onRetry={() => { void settings.refetch(); retry(); }} />
      </SignedOutFrame>
    ) : (
      <SignedOutFrame>
        <Loading />
      </SignedOutFrame>
    );
  }

  if (!settings.data.setupComplete) return <SetupPage locked={locked} />;

  const Page = pages[shown];
  return (
    <KoppelShell page={shown} role={role} counts={counts}>
      <Page />
    </KoppelShell>
  );
}
