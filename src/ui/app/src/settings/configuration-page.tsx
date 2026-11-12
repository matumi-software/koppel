import { Note, Panel, PanelBody, PanelNote, RoleCards, Setting, SettingList, Switch } from '@koppel/react-ui';
import { ChangePassword } from './change-password';
import { ConnectionSection } from '@/client/connection-section';
import { Frame } from '@/shell/frame';
import { useHasTray, useMachineName } from '@/shared/use-machine-name';
import type { KoppelSettings } from '@/api/settings';
import { useFrame } from '@/shell/frame-context';
import { isClientRole, isServerRole, roleName, roleOptions } from './roles';
import { useSettings, type SettingStatus } from './use-settings';

/** The settings for the tray icon's notifications, which only the Windows app has. */
type NotifyKey = keyof Pick<KoppelSettings, 'notifyPairing' | 'notifyLink' | 'notifyAttachFailures'>;

/**
 * Everything this machine is configured with: the role first, then the settings that role uses,
 * the server's above the client's. Every change waits for the server.
 */
export function ConfigurationPage() {
  const host = useMachineName();
  const { locked } = useFrame();
  const { query, status, requestChange } = useSettings();
  const hasTray = useHasTray();

  if (!query.data) {
    return (
      <Frame title="Configuration">
        <Note>{query.isError ? 'Could not load settings. Check that Koppel is running, then reload.' : 'Loading settings…'}</Note>
      </Frame>
    );
  }

  const role = status('role')!;
  const share = status('shareOnStartup')!;
  const notify = (setting: NotifyKey, title: string, description: string) => {
    const value = status(setting)!;
    return (
      <Setting
        key={setting}
        title={title}
        description={description}
        control={
          <Switch
            label={title}
            labelHidden
            confirmed={value.confirmed}
            pendingTarget={value.pendingTarget}
            failure={value.failure}
            note={value.saved ? 'Saved' : undefined}
            isDisabled={locked}
            onRequestChange={(next) => requestChange(setting, next)}
          />
        }
      />
    );
  };

  return (
    <Frame title="Configuration" lede={`${host}: what it does on the network and how it starts.`}>
      <Panel title="Role" meta={<SavedNote status={role} />}>
        <PanelBody>
          {!roleOptions.some((o) => o.value === role.confirmed) && (
            <Note>This machine's role is {roleName(role.confirmed)}: this version of Koppel does not know it. Pick one below to replace it.</Note>
          )}
          <RoleCards
            label="Role"
            options={roleOptions}
            value={role.confirmed}
            pendingValue={role.pendingTarget}
            isDisabled={locked}
            onChange={(value) => value !== role.confirmed && requestChange('role', value)}
          />
        </PanelBody>
      </Panel>

      {isServerRole(role.confirmed) && (
        <Panel title="Sharing">
          <SettingList label="Sharing">
            <Setting
              title="Share saved devices on startup"
              description="When Koppel starts, the devices you shared before are shared again."
              control={
                <Switch
                  label="Share saved devices on startup"
                  labelHidden
                  confirmed={share.confirmed}
                  pendingTarget={share.pendingTarget}
                  failure={share.failure}
                  note={share.saved ? 'Saved' : undefined}
                  isDisabled={locked}
                  onRequestChange={(value) => requestChange('shareOnStartup', value)}
                />
              }
            />
          </SettingList>
        </Panel>
      )}

      {isClientRole(role.confirmed) && <ConnectionSection locked={locked} />}

      {hasTray && (
        <Panel title="Notifications">
          <SettingList label="Notifications">
            {isServerRole(role.confirmed) &&
              notify('notifyPairing', 'When a client wants to pair', 'A Windows notification names the client. Click it to open Clients and type its code.')}
            {isClientRole(role.confirmed) &&
              notify('notifyLink', 'When the link to the server drops', 'Once when it drops and once when it is back, not on every retry.')}
            {isClientRole(role.confirmed) &&
              notify('notifyAttachFailures', 'When a device does not attach', 'Names the device and why. Koppel keeps trying by itself.')}
          </SettingList>
        </Panel>
      )}

      <ChangePassword locked={locked} />
    </Frame>
  );
}

/** Saving, Saved or why not, for a setting the panel holds. Nothing until the user changes it. */
function SavedNote({ status }: { status: SettingStatus<'role'> }) {
  const [state, text] =
    status.pendingTarget !== undefined
      ? (['pending', 'Saving…'] as const)
      : status.failure
        ? (['fault', `Not saved: ${status.failure}`] as const)
        : status.saved
          ? ([undefined, 'Saved'] as const)
          : ([undefined, undefined] as const);
  return <PanelNote state={state}>{text}</PanelNote>;
}
