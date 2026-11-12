import { useQuery } from '@tanstack/react-query';
import { useMemo, useState } from 'react';
import { AppShell, LinkDot, NavGroup, NavItem, Note, StatusCard, useAnnounce } from '@koppel/react-ui';
import { ClientDevicesPage } from '@/devices/client-devices';
import { ServerDevicesPage } from '@/devices/server-devices';
import { hostOf, useClientDevices, useServerDevices } from '@/devices/use-devices';
import { useAuth } from '@/auth/auth-context';
import { useLive } from '@/live/live-context';
import { OverviewPage } from '@/overview/overview-page';
import { ClientsPage } from '@/server/clients-page';
import { ConfigurationPage } from '@/settings/configuration-page';
import { isClientRole, isServerRole } from '@/settings/roles';
import { SetupPage } from '@/setup/setup-page';
import { storeAsideWidth, storedAsideWidth } from '@/shell/aside-width';
import { usePending } from '@/pending/pending-context';
import { setupKey } from '@/pending/domains';
import { FrameContext, pageTitles, usePage, type Page } from '@/shell/frame-context';
import { reasonOf } from '@/errors';
import { formatVersion, serverLine } from '@/client/server-link';
import { useActivity } from '@/activity/activity-context';
import { useKoppelVersion, useMachineName } from '@/shared/use-machine-name';
import { clientsOptions, connectionOptions, settingsOptions } from '@/queries';

export function App() {
  const host = useMachineName();
  const version = useKoppelVersion();
  const { state: link, retry } = useLive();
  const { signOut } = useAuth();
  const { items } = useActivity();
  const [isMenuOpen, setMenuOpen] = useState(false);
  const [signOutFailure, setSignOutFailure] = useState<string>();
  const [asideSlot, setAsideSlot] = useState<HTMLDivElement | null>(null);
  const announce = useAnnounce();
  const locked = link !== 'live';

  const settings = useQuery(settingsOptions());
  // First run lands on the page the new role needs first, once the server has confirmed setup. After
  // that, an address with no page is the overview.
  const { completed } = usePending('settings');
  const justSetUp = (completed[setupKey] ?? 0) > 0;
  const page = usePage(justSetUp ? (settings.data?.role === 'client' ? 'configuration' : 'clients') : 'overview');
  // Until the settings load, and for a role this UI does not know, the machine is neither.
  const isServer = isServerRole(settings.data?.role);
  const isClient = isClientRole(settings.data?.role);
  const server = useServerDevices(isServer && !!settings.data?.setupComplete);
  const client = useClientDevices(isClient && !!settings.data?.setupComplete);
  const clients = useQuery({ ...clientsOptions(), enabled: isServer });
  const connection = useQuery({ ...connectionOptions(), enabled: isClient });
  const remote = hostOf(connection.data?.serverUrl);

  // A page that does not exist for this role falls back to the overview, once the role is known:
  // before that a bookmarked #attach would be thrown away on every load.
  const allowed: Page[] = ['overview', 'configuration', ...(isServer ? (['devices', 'clients'] as Page[]) : []), ...(isClient ? (['attach'] as Page[]) : [])];
  const shown: Page = !settings.data || allowed.includes(page) ? page : 'overview';

  const pendingCount = items.filter((i) => i.state === 'pending').length;
  const faultCount =
    (server.data?.devices ?? []).filter((d) => d.fault).length + (client.data?.devices ?? []).filter((e) => e.fault).length;
  const needs = pendingCount + faultCount;

  const closeMenu = () => setMenuOpen(false);
  const item = (page: Page) => ({ href: `#${page}`, isCurrent: shown === page, onPress: closeMenu });

  const nav = (
    <>
      <NavGroup>
        <NavItem {...item('overview')} count={needs || undefined} countTone="pending">
          Overview
        </NavItem>
      </NavGroup>
      {isServer && (
        <NavGroup label={`Sharing from ${host}`}>
          <NavItem {...item('devices')} count={server.data?.devices.length}>
            Devices
          </NavItem>
          <NavItem {...item('clients')} count={clients.data?.length}>
            Clients
          </NavItem>
        </NavGroup>
      )}
      {isClient && (
        <NavGroup label="Attaching">
          <NavItem {...item('attach')} count={client.data?.devices.length}>
            {remote ? `From ${remote}` : 'Devices'}
          </NavItem>
        </NavGroup>
      )}
      <NavGroup label="Settings">
        <NavItem {...item('configuration')}>Configuration</NavItem>
      </NavGroup>
    </>
  );

  const navFoot = (
    <StatusCard
      state={link}
      machineName={host}
      version={formatVersion(version)}
      detail={isClient && client.data ? serverLine(client.data.link, remote) : undefined}
      onRetry={retry}
      onSignOut={() => {
        setSignOutFailure(undefined);
        signOut().catch((error: unknown) => {
          const reason = reasonOf(error, 'the request failed.');
          setSignOutFailure(reason);
          announce(`Signing out failed: ${reason}`);
        });
      }}
      signOutError={signOutFailure}
    />
  );

  const frame = useMemo(() => ({ locked, asideSlot }), [locked, asideSlot]);

  if (!settings.data) {
    return <Note isPadded>{settings.isError ? 'Could not reach Koppel. Check that it is running, then reload.' : 'Loading…'}</Note>;
  }

  if (!settings.data.setupComplete) {
    return <SetupPage locked={locked} />;
  }

  // One shell for every page, so the side panel is one panel too: pages swap its content, never its width.
  return (
    <FrameContext.Provider value={frame}>
      <AppShell
        nav={nav}
        navFoot={navFoot}
        asideRef={setAsideSlot}
        defaultAsideWidth={storedAsideWidth()}
        onAsideWidthChange={storeAsideWidth}
        title={pageTitles[shown]}
        status={<LinkDot state={link} />}
        isMenuOpen={isMenuOpen}
        onMenuOpenChange={setMenuOpen}
      >
        {shown === 'overview' && <OverviewPage />}
        {shown === 'devices' && <ServerDevicesPage />}
        {shown === 'clients' && <ClientsPage />}
        {shown === 'attach' && <ClientDevicesPage />}
        {shown === 'configuration' && <ConfigurationPage />}
      </AppShell>
    </FrameContext.Provider>
  );
}
