import { Button, KoppelMark, LinkBanner } from '@koppel/react-ui';
import { useHub } from '@/hub/hub-context';
import { SettingsPage } from '@/settings/settings-page';

const serverName = window.location.hostname;

export function App() {
  const { state, retry } = useHub();

  return (
    <div className="app">
      <header className="app-header">
        <div className="app-brand">
          <KoppelMark size={24} />
          <span className="kp-type-heading">Koppel</span>
        </div>
        {state === 'live' && <LinkBanner state="live" serverName={serverName} />}
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
      <main className="app-main">
        <SettingsPage />
      </main>
    </div>
  );
}
