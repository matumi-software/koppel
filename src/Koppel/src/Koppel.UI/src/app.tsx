import { useState } from 'react';
import { Button, KoppelMark, LinkBanner, SegmentedControl } from '@koppel/react-ui';
import { DevicesPage } from '@/devices/devices-page';
import { useHub } from '@/hub/hub-context';
import { SettingsPage } from '@/settings/settings-page';

const serverName = window.location.hostname;

type View = 'devices' | 'settings';

const views = [
  { value: 'devices', label: 'Devices' },
  { value: 'settings', label: 'Settings' },
] as const;

export function App() {
  const { state, retry } = useHub();
  const [view, setView] = useState<View>('devices');

  return (
    <div className="app">
      <header className="app-header">
        <div className="app-brand">
          <KoppelMark size={24} />
          <span className="kp-type-heading">Koppel</span>
        </div>
        <SegmentedControl<View> label="View" options={views} value={view} onChange={setView} />
        {state === 'live' ? <LinkBanner state="live" serverName={serverName} /> : <span />}
      </header>
      {state !== 'live' && (
        <div className="app-band">
          <LinkBanner
            state={state}
            serverName={serverName}
            retry={
              <Button variant="secondary" onPress={retry}>
                Retry
              </Button>
            }
          />
        </div>
      )}
      <main className="app-main">{view === 'devices' ? <DevicesPage /> : <SettingsPage />}</main>
    </div>
  );
}
