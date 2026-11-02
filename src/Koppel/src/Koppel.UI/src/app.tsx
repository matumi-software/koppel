import { useQuery } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import { Button, LinkBanner, NavGroup, NavItem } from '@koppel/react-ui';
import { getClients } from '@/api/server';
import { getConnection } from '@/api/client';
import { getSettings } from '@/api/settings';
import { ConnectionPage } from '@/client/connection-page';
import { ClientDevicesPage } from '@/devices/client-devices';
import { ServerDevicesPage } from '@/devices/server-devices';
import { hostOf, useClientDevices, useServerDevices } from '@/devices/use-devices';
import { useHub } from '@/hub/hub-context';
import { OverviewPage } from '@/overview/overview-page';
import { ClientsPage } from '@/server/clients-page';
import { MachinePage } from '@/settings/machine-page';
import { SetupPage } from '@/setup/setup-page';
import { FrameContext, usePageFromHash, type Page } from '@/shell/frame-context';
import { useActivity } from '@/activity/activity-context';
import { useMachineName } from '@/shared/use-machine-name';

export function App() {
  const host = useMachineName();
  const { state: link, retry } = useHub();
  const { items } = useActivity();
  const [page, go] = usePageFromHash();
  const [isMenuOpen, setMenuOpen] = useState(false);
  const locked = link !== 'live';

  const settings = useQuery({ queryKey: ['settings'], queryFn: getSettings });
  const role = settings.data?.role ?? 'server';
  const isServer = role !== 'client';
  const isClient = role !== 'server';
  const server = useServerDevices(isServer && !!settings.data?.setupComplete);
  const client = useClientDevices(isClient && !!settings.data?.setupComplete);
  const clients = useQuery({ queryKey: ['clients'], queryFn: getClients, enabled: isServer });
  const connection = useQuery({ queryKey: ['connection'], queryFn: getConnection, enabled: isClient });
  const remote = hostOf(connection.data?.serverUrl);

  // A page that does not exist for this role falls back to the overview.
  const allowed: Page[] = ['overview', 'machine', ...(isServer ? (['devices', 'clients'] as Page[]) : []), ...(isClient ? (['attach', 'connection'] as Page[]) : [])];
  const shown: Page = allowed.includes(page) ? page : 'overview';

  const pendingCount = items.filter((i) => i.state === 'pending').length;
  const faultCount =
    (server.data?.devices ?? []).filter((d) => d.fault).length + (client.data?.devices ?? []).filter((e) => e.fault).length;
  const needs = pendingCount + faultCount;

  const open = (next: Page) => {
    go(next);
    setMenuOpen(false);
  };

  useEffect(() => {
    if (page !== shown) go(shown);
  }, [page, shown, go]);

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
        <NavItem isCurrent={shown === 'machine'} onPress={() => open('machine')}>
          This machine
        </NavItem>
        {isClient && (
          <NavItem isCurrent={shown === 'connection'} onPress={() => open('connection')}>
            Server connection
          </NavItem>
        )}
      </NavGroup>
    </>
  );

  const navFoot = (
    <>
      <LinkBanner
        state={link}
        serverName={host}
        retry={
          <Button variant="secondary" onPress={retry}>
            Retry
          </Button>
        }
      />
      {isClient && client.data && (
        <span className="kp-type-caption" style={{ color: client.data.link === 'connected' ? 'var(--text-tertiary)' : 'var(--pending-text)' }}>
          {client.data.link === 'connected'
            ? `Linked to ${remote}.`
            : client.data.link === 'unconfigured'
              ? 'No server connection yet.'
              : client.data.link === 'connecting'
                ? `Connecting to ${remote ?? 'the server'}…`
                : `Cannot reach ${remote ?? 'the server'}.`}
        </span>
      )}
    </>
  );

  const status = (
    <span className="kp-row" role="status" aria-label={link === 'live' ? 'Live' : link === 'reconnecting' ? 'Reconnecting' : 'Disconnected'}>
      <span className="kp-glyph" data-state={link === 'live' ? 'live' : link === 'reconnecting' ? 'pending' : 'fault'} aria-hidden="true" />
    </span>
  );

  const frame = useMemo(
    () => ({ page: shown, go: open, nav, navFoot, navFootState: link, status, isMenuOpen, setMenuOpen, locked }),
    // The nav and foot are rebuilt every render anyway; memo only keeps the object stable between them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [shown, nav, navFoot, link, status, isMenuOpen, locked],
  );

  if (!settings.data) {
    return <p className="kp-note" style={{ padding: 24 }}>{settings.isError ? 'Could not reach Koppel. Check that it is running, then reload.' : 'Loading…'}</p>;
  }

  if (!settings.data.setupComplete) {
    return <SetupPage locked={locked} onDone={(chosen) => go(chosen === 'client' ? 'connection' : 'clients')} />;
  }

  return (
    <FrameContext.Provider value={frame}>
      {shown === 'overview' && <OverviewPage />}
      {shown === 'devices' && <ServerDevicesPage />}
      {shown === 'clients' && <ClientsPage />}
      {shown === 'attach' && <ClientDevicesPage />}
      {shown === 'machine' && <MachinePage />}
      {shown === 'connection' && <ConnectionPage />}
    </FrameContext.Provider>
  );
}
