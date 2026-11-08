import { Button } from '@koppel/react-ui';
import { AuthFrame } from './auth-frame';

/**
 * No password was set while setup was open. Koppel takes the first one only for a while after it
 * starts, so the way on is a restart, then this page again.
 */
export function SetupClosedPage({ machineName, recheck }: { machineName: string; recheck: () => void }) {
  return (
    <AuthFrame>
      <div className="kp-page-head">
        <h1>Setup has timed out</h1>
        <p>
          Koppel takes the first password for {machineName} only for a few minutes after it starts, so a stranger cannot claim a machine left waiting.
          Restart Koppel, then open this page again.
        </p>
      </div>
      <section className="kp-card" aria-label="How to restart Koppel">
        <h2>How to restart Koppel</h2>
        <ul className="kp-note">
          <li>
            In Docker: <code>docker compose restart koppel</code>
          </li>
          <li>
            Linux package: <code>sudo systemctl restart koppel</code>
          </li>
          <li>
            Windows app: right-click the Koppel icon in the notification area and pick <strong>Restart Koppel</strong>
          </li>
        </ul>
      </section>
      <div className="kp-row" style={{ justifyContent: 'flex-end' }}>
        <Button variant="primary" onPress={recheck}>
          Check again
        </Button>
      </div>
    </AuthFrame>
  );
}
