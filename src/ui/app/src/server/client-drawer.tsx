import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';
import { ActivityFeed, Button, Caption, Chips, CopyValue, Drawer, DrawerSection, LabeledValue, ModalDialog, Note, Row, SecretValue, useAnnounce } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import { getClientSecret, rotateClientSecret, type ClientView } from '@/api/server';
import { reasonOf } from '@/errors';
import { useNow } from '@/shared/use-now';
import { seen } from './seen';

const dateFormat = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });

interface ClientDrawerProps {
  client: ClientView;
  attached: string[];
  /** A new certificate is prepared, so the drawer says whether this client has it. */
  handover: boolean;
  isRevoking: boolean;
  revokeFailure?: string;
  locked: boolean;
  onRevoke: () => void;
  onClose: () => void;
}

/** A client's details in the aside: ID, secret, what it uses and its history; rotate or revoke it here. */
export function ClientDrawer({ client, attached, handover, isRevoking, revokeFailure, locked, onRevoke, onClose }: ClientDrawerProps) {
  const now = useNow();
  const { items } = useActivity();
  const announce = useAnnounce();
  const [confirmRotate, setConfirmRotate] = useState(false);
  const [confirmRevoke, setConfirmRevoke] = useState(false);
  // The new secret is shown once, here, until the drawer closes: it is the mutation's own result.
  const rotate = useMutation({
    mutationFn: () => rotateClientSecret(client.id),
    onSuccess: () => {
      setConfirmRotate(false);
      announce(`New secret for ${client.name}`);
    },
    onError: (error) => announce(`Not rotated: ${reasonOf(error, 'the request failed.')}`),
  });
  const rotated = rotate.data;

  return (
    <Drawer
      title={client.name}
      subtitle={
        <Caption tone="faint">
          Added {dateFormat.format(new Date(client.createdAt))} · {seen(client, now)}
        </Caption>
      }
      onClose={onClose}
    >
      <LabeledValue label="Client ID">
        <CopyValue value={client.id} />
      </LabeledValue>
      <LabeledValue label="Client secret" help={rotated ? `New secret. Put it on ${client.name} now; the old one no longer works.` : undefined}>
        {rotated ? (
          <CopyValue value={rotated} wrap />
        ) : (
          <SecretValue
            key={client.hasSecret ? 'kept' : 'missing'}
            label="Client secret"
            loadSecret={() => getClientSecret(client.id)}
            unavailable={client.hasSecret ? undefined : 'This client was added before Koppel kept secrets. Rotate the secret to get one you can copy.'}
          />
        )}
      </LabeledValue>

      <LabeledValue label="Using" help={attached.length ? undefined : 'No devices attached right now.'}>
        {attached.length > 0 && <Chips items={attached} />}
      </LabeledValue>

      {handover && (
        <LabeledValue
          label="New certificate"
          help={
            client.hasNextCertificate
              ? `${client.name} has its fingerprint and moves to it when you switch.`
              : `${client.name} has not received its fingerprint yet. It gets it the next time it connects.`
          }
        />
      )}

      <Row>
        <Button variant="secondary" isDisabled={locked} onPress={() => setConfirmRotate(true)}>
          Rotate secret
        </Button>
        <Button variant="danger" isSending={isRevoking} isDisabled={locked} onPress={() => setConfirmRevoke(true)}>
          {isRevoking ? 'Revoking' : 'Revoke'}
        </Button>
      </Row>
      {revokeFailure && <Note tone="error">Not revoked: {revokeFailure}</Note>}

      <DrawerSection title="History">
        <ActivityFeed entries={toEntries(items, client.id)} empty="Nothing about this client yet." />
      </DrawerSection>

      <ModalDialog isOpen={confirmRotate} onOpenChange={setConfirmRotate} title={`Rotate the secret for ${client.name}?`}>
        <Note>{client.name} stops getting new tokens until you give it the new secret. Devices it has attached stay attached until it reconnects.</Note>
        {rotate.isError && <Note tone="error">Not rotated: {reasonOf(rotate.error, 'the request failed.')}</Note>}
        <Row justify="end">
          <Button variant="secondary" slot="close">
            Cancel
          </Button>
          <Button variant="primary" isSending={rotate.isPending} isDisabled={locked} onPress={() => rotate.mutate()}>
            Rotate secret
          </Button>
        </Row>
      </ModalDialog>

      <ModalDialog isOpen={confirmRevoke} onOpenChange={setConfirmRevoke} title={`Revoke ${client.name}?`}>
        <Note>{client.name} can no longer get tokens or attach devices. To let it back, add it again and give it the new ID and secret.</Note>
        <Row justify="end">
          <Button variant="secondary" slot="close">
            Cancel
          </Button>
          <Button
            variant="danger"
            isDisabled={locked}
            onPress={() => {
              setConfirmRevoke(false);
              onRevoke();
            }}
          >
            Revoke
          </Button>
        </Row>
      </ModalDialog>
    </Drawer>
  );
}
