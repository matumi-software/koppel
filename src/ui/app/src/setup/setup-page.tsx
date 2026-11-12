import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Caption, Card, FirstRunShell, Note, PageHead, RoleCards, Row, SetupChecks, Steps } from '@koppel/react-ui';
import type { KoppelRole } from '@/api/settings';
import { roleOptions } from '@/settings/roles';
import { useSettings } from '@/settings/use-settings';
import { useMachineName } from '@/shared/use-machine-name';
import { systemChecksOptions } from '@/queries';

/**
 * First run: pick what this machine does, and see whether its kernel can do it. Continue saves the
 * role and marks setup done; the page leaves only when the server confirms both.
 */
export function SetupPage({ locked }: { locked: boolean }) {
  const host = useMachineName();
  const { query, requestSetup, setup } = useSettings();
  const checks = useQuery(systemChecksOptions());
  const [role, setRole] = useState<KoppelRole | null>(null);
  const chosen = role ?? query.data?.role ?? 'server';
  const pending = setup.isPending;
  const failure = setup.failure;

  const needed = chosen === 'server' ? ['share'] : chosen === 'client' ? ['attach'] : ['share', 'attach'];
  const blocking = (checks.data ?? []).filter((c) => needed.includes(c.id) && !c.ok);

  return (
    <FirstRunShell title="Set up" width="wide">
      <Steps steps={['Admin password', 'What this machine does', chosen === 'client' ? 'Server connection' : 'Clients']} current={1} />
      <PageHead title={`Set up ${host}`} lede="Pick what this machine does. You can change it later under Configuration." />

      <RoleCards label="Role" options={roleOptions} value={chosen} isDisabled={locked || pending} onChange={setRole} />

      <Card label="Checking this machine">
        <h2>Checking this machine</h2>
        {checks.data ? (
          <SetupChecks
            checks={checks.data
              .filter((c) => needed.includes(c.id))
              .map((c) => ({ id: c.id, ok: c.ok, title: c.title, detail: c.detail }))}
          />
        ) : (
          <Note>Checking…</Note>
        )}
      </Card>

      {failure && <Note tone="error">Not saved: {failure}</Note>}
      <Row justify="between">
        <Caption tone={blocking.length ? 'fault' : 'faint'}>
          {blocking.length ? 'Koppel can save this role, but it cannot use USB until the problems above are fixed.' : 'Everything this role needs is here.'}
        </Caption>
        <Button variant="primary" isSending={pending} isDisabled={locked} onPress={() => requestSetup(chosen)}>
          {pending ? 'Saving' : 'Continue'}
        </Button>
      </Row>
    </FirstRunShell>
  );
}
