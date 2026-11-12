import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';
import { Button, Card, CopyValue, Field, Form, LabeledValue, ModalDialog, Note, Row, useAnnounce } from '@koppel/react-ui';
import { createClient, type NewClient } from '@/api/server';
import { useServerErrors } from '@/errors';

/** Server role: a name in, a client ID and secret out, shown once in a dialog and kept on the server for later. */
export function AddClient({ locked, onAdded }: { locked: boolean; onAdded: (id: string) => void }) {
  const [name, setName] = useState('');
  const [created, setCreated] = useState<NewClient>();
  const announce = useAnnounce();
  const add = useMutation({
    mutationFn: createClient,
    onSuccess: (client) => {
      setCreated(client);
      setName('');
      onAdded(client.clientId);
      announce(`${client.name} is added`);
    },
  });
  const errors = useServerErrors(add.error, 'name', 'the request failed.');

  return (
    <Card label="Add a client" gap="sm">
      <Form
        label="Add a client"
        validationErrors={errors}
        onSubmit={(event) => {
          event.preventDefault();
          if (!add.isPending && name.trim() !== '') add.mutate(name);
        }}
      >
        <Field
          label="New client"
          name="name"
          placeholder="Machine name, such as bench-2"
          value={name}
          onChange={setName}
          isDisabled={locked || add.isPending}
          help="You get a client ID and a secret. You can copy both again later from the client."
          action={
            <Button variant="primary" type="submit" isSending={add.isPending} isDisabled={locked || name.trim() === ''}>
              Add client
            </Button>
          }
        />
      </Form>
      <ModalDialog isOpen={!!created} onOpenChange={(open) => !open && setCreated(undefined)} title={`${created?.name ?? 'Client'} is added`}>
        {created && (
          <>
            <Note>Put the ID and secret on {created.name}, under Configuration, Server connection, with the server fingerprint.</Note>
            <LabeledValue label="Client ID">
              <CopyValue value={created.clientId} />
            </LabeledValue>
            <LabeledValue label="Client secret">
              <CopyValue value={created.clientSecret} wrap />
            </LabeledValue>
            <Row justify="end">
              <Button variant="primary" slot="close">
                Done
              </Button>
            </Row>
          </>
        )}
      </ModalDialog>
    </Card>
  );
}
