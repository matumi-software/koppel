import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, PageHeader, RadioGroup, StatusDot } from '@matumi/ui';
import type { Role } from '@/api/settings';
import { roleOptions } from '@/settings/roles';
import { useSettings } from '@/settings/use-settings';
import { useMachineName } from '@/shared/use-machine-name';
import { SignedOutFrame } from '@/shell/signed-out-frame';
import { systemChecksOptions } from '@/queries';

/**
 * First run, step two: pick what this computer does, and see whether it can do it. Continue saves
 * the role and marks setup done; the page leaves only when the server confirms both.
 */
export function SetupPage({ locked }: { locked: boolean }) {
  const host = useMachineName();
  const { query, requestSetup, setup } = useSettings();
  const checks = useQuery(systemChecksOptions());
  const [role, setRole] = useState<Role | null>(null);
  const chosen = role ?? query.data?.role ?? 'server';

  const needed = chosen === 'server' ? ['share'] : chosen === 'client' ? ['attach'] : ['share', 'attach'];
  const shown = (checks.data ?? []).filter((c) => needed.includes(c.id));
  const blocking = shown.filter((c) => !c.ok);

  return (
    <SignedOutFrame>
      <div className="kp-page kp-form">
        <PageHeader title={`What does ${host} do?`} description="Step 2 of 2. You can change this later in Settings." />
        <RadioGroup
          label="Role"
          value={chosen}
          isDisabled={locked || setup.isPending}
          onChange={(value) => setRole(value as Role)}
          items={roleOptions.map(({ value, label }) => ({ value, label }))}
        />
        <div className="kp-stack-1">
          <span className="hw-label">This computer has</span>
          {checks.data ? (
            shown.map((c) => (
              <StatusDot key={c.id} tone={c.ok ? 'success' : 'danger'}>
                {c.ok ? c.title : `${c.title}. ${c.detail}`}
              </StatusDot>
            ))
          ) : (
            <span className="kp-muted">Checking…</span>
          )}
        </div>
        {blocking.length > 0 && <p className="kp-muted">Koppel can save this role, but it cannot use USB until the problems above are fixed.</p>}
        {setup.failure && <p className="kp-danger">Not saved: {setup.failure}</p>}
        <div>
          <Button variant="primary" isPending={setup.isPending} isDisabled={locked} onPress={() => requestSetup(chosen)}>
            Continue
          </Button>
        </div>
      </div>
    </SignedOutFrame>
  );
}
