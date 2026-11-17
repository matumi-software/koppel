import { Button, Facts, PageHeader } from '@heartwood/ui';
import { SignedOutFrame } from '@/shell/signed-out-frame';

/**
 * No password was set while setup was open. Koppel takes the first one only for a while after it
 * starts, so the way on is a restart, then this page again.
 */
export default function SetupClosedPage({ machineName, recheck }: { machineName: string; recheck: () => void }) {
  return (
    <SignedOutFrame>
      <div className="kp-page kp-text">
        <PageHeader
          title="Setup has timed out"
          description={`For safety, Koppel takes the first password for ${machineName} only for a short while after it starts.`}
        />
        <p className="kp-muted">Restart Koppel, then open this page again within that time.</p>
        <Facts
          label="How to restart Koppel"
          items={[
            { label: 'Docker', value: 'docker compose restart koppel', path: true, copy: true },
            { label: 'Linux package', value: 'sudo systemctl restart koppel', path: true, copy: true },
            { label: 'Windows', value: 'Restart-Service Koppel', path: true, copy: true },
          ]}
        />
        <div>
          <Button variant="primary" onPress={recheck}>
            Check again
          </Button>
        </div>
      </div>
    </SignedOutFrame>
  );
}
