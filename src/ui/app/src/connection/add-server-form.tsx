import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Button, TextField } from '@heartwood/ui';
import { RejectedError } from '@/api/http';
import { standalone } from '@/errors';
import { discoveredServersOptions } from '@/queries';
import { useAnnounce } from '@/shared/announce-context';
import { useFrame } from '@/shell/frame-context';
import { useAddServer, useServers } from './use-servers';

/**
 * Adds a server by its address and a pairing code or the server key. The request answers 201 with the
 * saved server, so a refusal is read here, not waited for: the server's own sentence goes under the
 * field it is about. Servers found on the network fill the address.
 */
export function AddServerForm({ first, onClose }: { first: boolean; onClose: () => void }) {
  const { locked } = useFrame();
  const announce = useAnnounce();
  const found = useQuery(discoveredServersOptions()).data ?? [];
  const saved = useServers().data?.servers ?? [];
  const [address, setAddress] = useState('');
  const [secret, setSecret] = useState('');
  const [useKey, setUseKey] = useState(false);
  const add = useAddServer();

  const problem = add.error instanceof RejectedError ? add.error.problem : undefined;
  const fields = problem?.errors;
  const credentialField = useKey ? 'key' : 'code';
  const message = add.error ? standalone(add.error.message) : undefined;
  const addressError = fields?.address?.[0] ?? (problem?.code === 'already_added' ? message : undefined);
  const credentialError = fields?.[credentialField]?.[0] ?? (message && !addressError && !fields ? message : undefined);
  const offered = found.filter((d) => !saved.some((s) => s.serverUrl === d.url));

  const edit = (set: (value: string) => void) => (value: string) => {
    set(value);
    if (add.isError) add.reset();
  };
  const submit = () => {
    const base = { address: address.trim() };
    add.mutate(useKey ? { ...base, key: secret.trim() } : { ...base, code: secret.replace(/\s/g, '') }, {
      onSuccess: (server) => {
        announce(`${server.name ?? 'The server'} is added`);
        onClose();
      },
    });
  };

  return (
    <form
      className="kp-form"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <TextField
        label="Server address"
        placeholder="lab-pi.local:5241"
        mono
        value={address}
        onChange={edit(setAddress)}
        description="Shown on the server, under Clients."
        isInvalid={!!addressError}
        errorMessage={addressError}
      />
      {offered.length ? (
        <div className="kp-stack-2">
          <span className="kp-muted kp-small">Found on this network. Pick one to fill in its address.</span>
          <div className="kp-actions">
            {offered.map((d) => (
              <Button key={d.url} variant="secondary" onPress={() => edit(setAddress)(`${d.address}:${d.httpsPort}`)}>
                {`${d.name} (${d.address}:${d.httpsPort})`}
              </Button>
            ))}
          </div>
        </div>
      ) : null}
      {useKey ? (
        <TextField label="Server key" placeholder="kp1_..." mono value={secret} onChange={edit(setSecret)} isInvalid={!!credentialError} errorMessage={credentialError} />
      ) : (
        <TextField
          label="Pairing code"
          placeholder="8 digits"
          mono
          inputMode="numeric"
          value={secret}
          onChange={edit(setSecret)}
          description="Show a pairing code on the server. It works once, for 60 seconds."
          isInvalid={!!credentialError}
          errorMessage={credentialError}
        />
      )}
      <div className="kp-actions">
        <Button variant="primary" type="submit" isPending={add.isPending} isDisabled={locked || address.trim() === '' || secret.trim() === ''}>
          {first ? 'Connect' : 'Add server'}
        </Button>
        <Button
          variant="ghost"
          onPress={() => {
            setUseKey(!useKey);
            setSecret('');
            add.reset();
          }}
        >
          {useKey ? 'Use a pairing code' : 'Use the server key'}
        </Button>
        {first ? null : (
          <Button variant="ghost" onPress={onClose}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}
