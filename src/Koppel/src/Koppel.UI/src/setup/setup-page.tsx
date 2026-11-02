import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, KoppelMark, RoleCards, SetupChecks, Steps } from '@koppel/react-ui';
import { getSystemChecks } from '@/api/http';
import type { KoppelRole } from '@/api/settings';
import { roleOptions } from '@/settings/roles';
import { useSettings } from '@/settings/use-settings';
import { useMachineName } from '@/shared/use-machine-name';

/**
 * First run: pick what this machine does, and see whether its kernel can do it. Continue saves the
 * role and marks setup done; the page leaves only when the server confirms both.
 */
export function SetupPage({ locked, onDone }: { locked: boolean; onDone: (role: KoppelRole) => void }) {
  const host = useMachineName();
  const { query, status, requestChange } = useSettings();
  const checks = useQuery({ queryKey: ['system-checks'], queryFn: getSystemChecks });
  const [role, setRole] = useState<KoppelRole | null>(null);
  const chosen = role ?? query.data?.role ?? 'server';
  const pending = status('setupComplete')?.pendingTarget !== undefined || status('role')?.pendingTarget !== undefined;
  const failure = status('setupComplete')?.failure ?? status('role')?.failure;

  const needed = chosen === 'server' ? ['share'] : chosen === 'client' ? ['attach'] : ['share', 'attach'];
  const blocking = (checks.data ?? []).filter((c) => needed.includes(c.id) && !c.ok);

  const finish = () => {
    if (chosen !== query.data?.role) requestChange('role', chosen);
    requestChange('setupComplete', true);
    onDone(chosen);
  };

  return (
    <div className="kp-shell" style={{ flexDirection: 'column' }}>
      <header className="kp-row" style={{ padding: '14px 24px', borderBottom: '1px solid var(--border-default)', background: 'var(--bg-surface)' }}>
        <KoppelMark size={26} />
        <span className="kp-type-heading">Koppel</span>
      </header>
      <main style={{ display: 'flex', justifyContent: 'center', padding: '40px 16px' }}>
        <div style={{ width: '100%', maxWidth: 920, display: 'flex', flexDirection: 'column', gap: 28 }}>
          <Steps steps={['What this machine does', chosen === 'client' ? 'Server connection' : 'Clients']} current={0} />
          <div className="kp-page-head">
            <h1>Set up {host}</h1>
            <p>Pick what this machine does. You can change it later under Configuration.</p>
          </div>

          <RoleCards label="Role" options={roleOptions} value={chosen} isDisabled={locked || pending} onChange={setRole} />

          <section className="kp-card" aria-label="Checking this machine">
            <h2>Checking this machine</h2>
            {checks.data ? (
              <SetupChecks
                checks={checks.data
                  .filter((c) => needed.includes(c.id))
                  .map((c) => ({ id: c.id, ok: c.ok, title: c.title, detail: c.detail }))}
              />
            ) : (
              <p className="kp-note">Checking…</p>
            )}
          </section>

          {failure && <p className="kp-error">Not saved: {failure}</p>}
          <div className="kp-row" style={{ justifyContent: 'space-between' }}>
            <span className="kp-type-caption" style={{ color: blocking.length ? 'var(--fault-text)' : 'var(--text-tertiary)' }}>
              {blocking.length ? 'Koppel can save this role, but it cannot use USB until the problems above are fixed.' : 'Everything this role needs is here.'}
            </span>
            <Button variant="primary" isSending={pending} isDisabled={locked} onPress={finish}>
              {pending ? 'Saving' : 'Continue'}
            </Button>
          </div>
        </div>
      </main>
    </div>
  );
}
