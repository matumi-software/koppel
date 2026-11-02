import { useState } from 'react';
import { Button, Field, Panel, RoleCards, Setting, SettingList, Switch } from '@koppel/react-ui';
import { ConnectionSection } from '@/client/connection-section';
import { Frame } from '@/shell/frame';
import { useMachineName } from '@/shared/use-machine-name';
import { useFrame } from '@/shell/frame-context';
import { isClientRole, isServerRole, roleName, roleOptions } from './roles';
import { useSettings, type SettingStatus } from './use-settings';

/**
 * Everything this machine is configured with: the role first, then the settings that role uses,
 * the server's above the client's. Every change waits for the server.
 */
export function ConfigurationPage() {
  const host = useMachineName();
  const { locked } = useFrame();
  const { query, status, requestChange } = useSettings();

  if (!query.data) {
    return (
      <Frame title="Configuration">
        <p className="kp-note">{query.isError ? 'Could not load settings. Check that Koppel is running, then reload.' : 'Loading settings…'}</p>
      </Frame>
    );
  }

  const role = status('role')!;
  const share = status('shareOnStartup')!;

  return (
    <Frame title="Configuration" lede={`${host}: what it does on the network and how it starts.`}>
      <Panel title="Role" meta={<SavedNote status={role} />}>
        <div className="kp-panel__body">
          {!roleOptions.some((o) => o.value === role.confirmed) && (
            <p className="kp-note">This machine's role is {roleName(role.confirmed)}: this version of Koppel does not know it. Pick one below to replace it.</p>
          )}
          <RoleCards
            label="Role"
            options={roleOptions}
            value={role.confirmed}
            pendingValue={role.pendingTarget}
            isDisabled={locked}
            onChange={(value) => value !== role.confirmed && requestChange('role', value)}
          />
        </div>
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
            <ListenPortSetting
              key={status('listenPort')!.confirmed}
              locked={locked}
              status={status('listenPort')!}
              onSave={(port) => requestChange('listenPort', port)}
            />
          </SettingList>
        </Panel>
      )}

      {isClientRole(role.confirmed) && <ConnectionSection locked={locked} />}
    </Frame>
  );
}

/** Saving, Saved or why not, for a setting the panel holds. Nothing until the user changes it. */
function SavedNote({ status }: { status: SettingStatus<'role'> }) {
  // The live region stays in the page, empty, so a screen reader hears the text arrive.
  const [state, text] =
    status.pendingTarget !== undefined
      ? ['pending', 'Saving…']
      : status.failure
        ? ['fault', `Not saved: ${status.failure}`]
        : status.saved
          ? [undefined, 'Saved']
          : [undefined, undefined];
  return (
    <span className="kp-panel__note" data-state={state} role="status">
      {text}
    </span>
  );
}

const validPort = (draft: string) => /^[0-9]{1,5}$/.test(draft) && Number(draft) >= 1 && Number(draft) <= 65535;

function ListenPortSetting({ locked, status, onSave }: { locked: boolean; status: SettingStatus<'listenPort'>; onSave: (port: number) => void }) {
  const [draft, setDraft] = useState(String(status.confirmed));
  const isPending = status.pendingTarget !== undefined;
  const isValid = validPort(draft);
  const isInvalid = !isValid && draft !== String(status.confirmed);
  const help = isInvalid ? 'Enter a whole number from 1 to 65535.' : status.failure ? `Not saved: ${status.failure}` : status.saved ? 'Saved' : undefined;
  return (
    <Setting
      title="Listen port"
      description="The port USB traffic arrives on. Encrypted, and only registered clients get in."
      control={
        <Field
          label="Listen port"
          labelHidden
          mono
          value={draft}
          onChange={setDraft}
          isDisabled={locked || isPending}
          state={isPending ? 'pending' : isInvalid || status.failure ? 'invalid' : undefined}
          help={help}
          action={
            <Button variant="secondary" isSending={isPending} isDisabled={locked || !isValid || draft === String(status.confirmed)} onPress={() => onSave(Number(draft))}>
              Save
            </Button>
          }
        />
      }
    />
  );
}
