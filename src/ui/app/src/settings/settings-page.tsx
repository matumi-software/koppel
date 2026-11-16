import { useQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { Select, SettingsPage as HeartwoodSettingsPage, Switch, type SettingsPageRow } from '@matumi/ui';
import type { KoppelSettings, Role } from '@/api/settings';
import { systemInfoOptions } from '@/queries';
import { useFrame } from '@/shell/frame-context';
import { Loading } from '@/shell/loading';
import { useLook } from '@/theme/theme';
import { notices } from './notices';
import { isClientRole, isServerRole, roleName } from './roles';
import { useSettings } from './use-settings';

type Toggle = keyof Pick<KoppelSettings, 'shareOnStartup' | 'notifyPairing' | 'notifyLink' | 'notifyAttachFailures'>;

const roleItems: { id: Role; label: string }[] = (['server', 'client', 'both'] as const).map((id) => ({ id, label: roleName(id) }));

/** The help under a row, with the server's reason when the last change was not saved. */
const helpWith = (text: string | undefined, failure: string | undefined): ReactNode =>
  failure ? (
    <>
      {text ? `${text} ` : null}
      <span className="kp-danger">Not saved: {failure}</span>
    </>
  ) : (
    text
  );

/**
 * This computer's settings on Heartwood's SettingsPage. Every change is sent at once and waits for
 * the settings the server pushes back (the confirm rule): the control shows pending until then.
 * Appearance is this browser's own and applies at once.
 */
export function SettingsPage() {
  const { locked } = useFrame();
  const { query, status, requestChange } = useSettings();
  const info = useQuery(systemInfoOptions());
  const { theme, colourVision, setTheme, setColourVision } = useLook();

  if (!query.data) return <Loading />;

  const name = info.data?.name ?? window.location.hostname;
  const role = status('role')!;
  const known = roleItems.some((item) => item.id === role.confirmed);

  const toggle = (key: Toggle, title: string, help?: string): SettingsPageRow => {
    const value = status(key)!;
    return {
      id: key,
      name: title,
      help: helpWith(help, value.failure),
      control: (
        <Switch
          aria-label={title}
          isSelected={value.confirmed}
          isPending={value.pendingTarget !== undefined}
          isDisabled={locked || value.pendingTarget !== undefined}
          onChange={(next) => requestChange(key, next)}
        />
      ),
    };
  };

  const roleHelp = known
    ? 'Share its own devices, attach devices from servers, or both.'
    : `This version of Koppel does not know the role ${role.confirmed}. Pick one to replace it.`;

  return (
    <HeartwoodSettingsPage
      description={`How ${name} runs Koppel.`}
      sections={[
        {
          title: 'This computer',
          rows: [
            {
              id: 'role',
              name: 'Role',
              help: helpWith(roleHelp, role.failure),
              control: (
                <Select
                  aria-label="Role"
                  items={roleItems}
                  selectedKey={known ? role.confirmed : null}
                  isPending={role.pendingTarget !== undefined}
                  isDisabled={locked || role.pendingTarget !== undefined}
                  onSelectionChange={(key) => key !== null && key !== role.confirmed && requestChange('role', key as Role)}
                  width="var(--size-width-xs)"
                />
              ),
            },
            ...(isServerRole(role.confirmed) ? [toggle('shareOnStartup', 'Share saved devices on startup', 'Shares again what was shared when Koppel last stopped.')] : []),
          ],
        },
        ...(info.data?.tray
          ? [
              {
                title: 'Notify me when',
                rows: [
                  ...(isServerRole(role.confirmed) ? [toggle('notifyPairing', `A computer links to ${name}`)] : []),
                  ...(isClientRole(role.confirmed) ? [toggle('notifyLink', 'The link to a server drops'), toggle('notifyAttachFailures', 'A device does not attach')] : []),
                ],
              },
            ]
          : []),
      ]}
      appearance={{ theme, onTheme: setTheme, vision: colourVision, onVision: setColourVision }}
      about={info.data && { product: 'Koppel', version: info.data.version, platform: info.data.os, notices }}
    />
  );
}
