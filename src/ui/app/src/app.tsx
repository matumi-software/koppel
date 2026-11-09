import { useQuery } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import { AppShell, NavGroup, NavItem, StatusCard } from '@koppel/react-ui';
import { getClients } from '@/api/server';
import { getConnection } from '@/api/client';
import { getSettings } from '@/api/settings';
import { ClientDevicesPage } from '@/devices/client-devices';
import { ServerDevicesPage } from '@/devices/server-devices';
import { hostOf, useClientDevices, useServerDevices } from '@/devices/use-devices';
import { useAuth } from '@/auth/auth-context';
import { useHub } from '@/hub/hub-context';
import { OverviewPage } from '@/overview/overview-page';
import { ClientsPage } from '@/server/clients-page';
import { ConfigurationPage } from '@/settings/configuration-page';
import { isClientRole, isServerRole } from '@/settings/roles';
import { SetupPage } from '@/setup/setup-page';
import { storeAsideWidth, storedAsideWidth } from '@/shell/aside-width';
import { FrameContext, usePageFromHash, type Page } from '@/shell/frame-context';
import { useActivity } from '@/activity/activity-context';
import { clientsKey, connectionKey, settingsKey } from '@/query-keys';
import { useKoppelVersion, useMachineName } from '@/shared/use-machine-name';

export function App() {
  const host = useMachineName();
  const version = useKoppelVersion();
  const { state: link, retry } = useHub();
  const { signOut } = useAuth();
  const { items } = useActivity();
  const [page, go] = usePageFromHash();
  const [isMenuOpen, setMenuOpen] = useState(false);
  const [signOutFailure, setSignOutFailure] = useState<string>();
  const [title, setTitle] = useState('Koppel');
  const [asideSlot, setAsideSlot] = useState<HTMLDivElement | null>(null);
  const locked = link !== 'live';

  const settings = useQuery({ queryKey: settingsKey, queryFn: getSettings });
  // Until the settings load, and for a role this UI does not know, the machine is neither.
  const isServer = isServerRole(settings.data?.role);
  const isClient = isClientRole(settings.data?.role);
  const server = useServerDevices(isServer && !!settings.data?.setupComplete);
  const client = useClientDevices(isClient && !!settings.data?.setupComplete);
  const clients = useQuery({ queryKey: clientsKey, queryFn: getClients, enabled: isServer });
  const connection = useQuery({ queryKey: connectionKey, queryFn: getConnection, enabled: isClient });
  const remote = hostOf(connection.data?.serverUrl);

  // A page that does not exist for this role falls back to the overview, once the role is known:
  // before that a bookmarked #attach would be thrown away on every load.
  const allowed: Page[] = ['overview', 'configuration', ...(isServer ? (['devices', 'clients'] as Page[]) : []), ...(isClient ? (['attach'] as Page[]) : [])];
  const shown: Page = !settings.data || allowed.includes(page) ? page : 'overview';

  const pendingCount = items.filter((i) => i.state === 'pending').length;
  const faultCount =
    (server.data?.devices ?? []).filter((d) => d.fault).length + (client.data?.devices ?? []).filter((e) => e.fault).length;
  const needs = pendingCount + faultCount;

  const open = (next: Page) => {
    go(next);
    setMenuOpen(false);
  };

  useEffect(() => {
    if (settings.data && page !== shown) go(shown);
  }, [settings.data, page, shown, go]);

  const nav = (
    <>
      <NavGroup>
        <NavItem isCurrent={shown === 'overview'} count={needs || undefined} countTone="pending" onPress={() => open('overview')}>
          Overview
        </NavItem>
      </NavGroup>
      {isServer && (
        <NavGroup label={`Sharing from ${host}`}>
          <NavItem isCurrent={shown === 'devices'} count={server.data?.devices.length} onPress={() => open('devices')}>
            Devices
          </NavItem>
          <NavItem isCurrent={shown === 'clients'} count={clients.data?.length} onPress={() => open('clients')}>
            Clients
          </NavItem>
        </NavGroup>
      )}
      {isClient && (
        <NavGroup label="Attaching">
          <NavItem isCurrent={shown === 'attach'} count={client.data?.devices.length} onPress={() => open('attach')}>
            {remote ? `From ${remote}` : 'Devices'}
          </NavItem>
        </NavGroup>
      )}
      <NavGroup label="Settings">
        <NavItem isCurrent={shown === 'configuration'} onPress={() => open('configuration')}>
          Configuration
        </NavItem>
      </NavGroup>
    </>
  );

  const navFoot = (
    <StatusCard
      state={link}
      machineName={host}
      version={version}
      server={isClient && client.data ? { link: client.data.link, name: remote } : undefined}
      onRetry={retry}
      onSignOut={() => {
        setSignOutFailure(undefined);
        signOut().catch((error: unknown) => setSignOutFailure(error instanceof Error ? error.message : 'the request failed.'));
      }}
      signOutError={signOutFailure}
    />
  );

  const status = (
    <span className="kp-row" role="status" aria-label={link === 'live' ? 'Live' : link === 'reconnecting' ? 'Reconnecting' : 'Disconnected'}>
      <span className="kp-glyph" data-state={link === 'live' ? 'live' : link === 'reconnecting' ? 'pending' : 'fault'} aria-hidden="true" />
    </span>
  );

  const frame = useMemo(
    () => ({ page: shown, go: open, nav, navFoot, navFootState: link, status, isMenuOpen, setMenuOpen, locked, asideSlot, setTitle }),
    // The nav and foot are rebuilt every render anyway; memo only keeps the object stable between them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [shown, nav, navFoot, link, status, isMenuOpen, locked, asideSlot],
  );

  if (!settings.data) {
    return <p className="kp-note" style={{ padding: 24 }}>{settings.isError ? 'Could not reach Koppel. Check that it is running, then reload.' : 'Loading…'}</p>;
  }

  if (!settings.data.setupComplete) {
    return <SetupPage locked={locked} onDone={(chosen) => go(chosen === 'client' ? 'configuration' : 'clients')} />;
  }

  // One shell for every page, so the side panel is one panel too: pages swap its content, never its width.
  return (
    <FrameContext.Provider value={frame}>
      <AppShell
        nav={nav}
        navFoot={navFoot}
        aside={<div ref={setAsideSlot} style={{ display: 'contents' }} />}
        defaultAsideWidth={storedAsideWidth()}
        onAsideWidthChange={storeAsideWidth}
        title={title}
        status={status}
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
