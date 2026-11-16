import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { Button, Callout, CopyValue, Dialog, TextField } from '@matumi/ui';
import { createClient, rotateClientSecret, type ClientView } from '@/api/server';
import { reasonOf } from '@/errors';
import { ConfirmDialog } from '@/shared/confirm-dialog';
import { useAnnounce } from '@/shared/announce-context';
import { useFrame } from '@/shell/frame-context';

/**
 * Replaces a client's secret. The commit is asked first; the new secret is the request's own answer,
 * shown once in this dialog and held nowhere else.
 */
export function RotateSecretDialog({ client, onClose }: { client: Pick<ClientView, 'id' | 'name'>; onClose: () => void }) {
  const { locked } = useFrame();
  const announce = useAnnounce();
  const rotate = useMutation({
    mutationFn: () => rotateClientSecret(client.id),
    onSuccess: () => announce(`New secret for ${client.name}`),
    onError: (error) => announce(`Not rotated: ${reasonOf(error, 'the request failed.')}`),
  });
  if (rotate.data) {
    return (
      <Dialog
        title={`New secret for ${client.name}`}
        size="sm"
        onClose={onClose}
        footer={(close) => (
          <Button variant="primary" onPress={close}>
            Done
          </Button>
        )}
      >
        <div className="kp-stack-3">
          <p className="kp-muted">Put it on {client.name} now. The old secret no longer works, and this is the only time it shows here.</p>
          <CopyValue value={rotate.data} label="New secret" masked revealable />
        </div>
      </Dialog>
    );
  }
  return (
    <ConfirmDialog title={`Rotate the secret of ${client.name}?`} commit="Rotate secret" danger keepOpen isDisabled={locked} isPending={rotate.isPending} onConfirm={() => rotate.mutate()} onClose={onClose}>
      <div className="kp-stack-3">
        <p className="kp-muted">{client.name} stops getting new tokens until you give it the new secret. Devices it has attached stay attached until it reconnects.</p>
        {rotate.isError ? (
          <Callout tone="danger" title="Not rotated">
            {reasonOf(rotate.error, 'the request failed.')}
          </Callout>
        ) : null}
      </div>
    </ConfirmDialog>
  );
}

/** Makes a client by hand, for a computer that cannot use a pairing code or the server key: a name in, an ID and secret out, once. */
export function AddClientDialog({ onClose }: { onClose: () => void }) {
  const { locked } = useFrame();
  const announce = useAnnounce();
  const [name, setName] = useState('');
  const add = useMutation({
    mutationFn: createClient,
    onSuccess: (created) => announce(`${created.name} is added`),
  });
  const created = add.data;
  const submit = () => {
    if (!add.isPending && name.trim() !== '') add.mutate(name.trim());
  };
  if (created) {
    return (
      <Dialog
        title={`${created.name} is added`}
        size="sm"
        onClose={onClose}
        footer={(close) => (
          <Button variant="primary" onPress={close}>
            Done
          </Button>
        )}
      >
        <div className="kp-stack-3">
          <p className="kp-muted">Put the ID and secret on {created.name}. You can copy the secret again later from the client.</p>
          <CopyValue value={created.clientId} label="Client ID" />
          <CopyValue value={created.clientSecret} label="Client secret" masked revealable />
        </div>
      </Dialog>
    );
  }
  const error = add.isError ? reasonOf(add.error, 'the request failed.') : undefined;
  return (
    <Dialog
      title="Add a client by hand"
      size="sm"
      onClose={onClose}
      footer={(close) => (
        <>
          <Button variant="secondary" onPress={close}>
            Cancel
          </Button>
          <Button variant="primary" isDisabled={locked || name.trim() === ''} isPending={add.isPending} onPress={submit}>
            Add client
          </Button>
        </>
      )}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <TextField label="Name" placeholder="Machine name, such as bench-2" value={name} onChange={setName} isInvalid={!!error} errorMessage={error} description="You get a client ID and a secret." autoFocus />
      </form>
    </Dialog>
  );
}
