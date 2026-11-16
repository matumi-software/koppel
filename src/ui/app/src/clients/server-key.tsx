import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { Button, Callout, CopyValue, IconButton } from '@heartwood/ui';
import { getEnrollmentKey, rotateEnrollmentKey } from '@/api/server';
import { reasonOf } from '@/errors';
import { ConfirmDialog } from '@/shared/confirm-dialog';
import { useAnnounce } from '@/shared/announce-context';
import { useFrame } from '@/shell/frame-context';

/** A key held only while it shows, with the revision of the server's key it was read at. */
interface Held {
  key: string;
  revision: number | undefined;
  /** Made here by rotating, so it is the current one whatever revision the push reports. */
  fresh: boolean;
}

/**
 * The server key as the control of its setting row: fetched only when someone asks to see it, then a
 * masked, revealable value with its copy button. The rotate icon button replaces the key and shows the
 * new one once. A key read at an older revision is dropped when the server's key changes.
 */
export function ServerKey({ revision }: { revision: number | undefined }) {
  const { locked } = useFrame();
  const announce = useAnnounce();
  const [held, setHeld] = useState<Held>();
  const [confirming, setConfirming] = useState(false);
  const show = useMutation({
    mutationFn: getEnrollmentKey,
    onSuccess: (key) => setHeld({ key, revision, fresh: false }),
    onError: (error) => announce(`Could not get the server key: ${reasonOf(error, 'the request failed.')}`),
  });
  const rotate = useMutation({
    mutationFn: rotateEnrollmentKey,
    onSuccess: (key) => {
      setHeld({ key, revision, fresh: true });
      announce('New server key');
    },
    onError: (error) => announce(`Not rotated: ${reasonOf(error, 'the request failed.')}`),
  });
  const visible = held && (held.fresh || held.revision === revision) ? held.key : undefined;
  return (
    <span className="kp-line">
      {visible ? (
        <CopyValue value={visible} label="Server key" masked revealable />
      ) : (
        <Button variant="secondary" isPending={show.isPending} onPress={() => show.mutate()}>
          Show
        </Button>
      )}
      <IconButton
        icon={RefreshCw}
        label="Rotate key"
        description="Computers linked with the old key keep working; the old key links no new ones."
        variant="secondary"
        isDisabled={locked}
        onPress={() => setConfirming(true)}
      />
      {confirming ? (
        <ConfirmDialog title="Rotate the server key?" commit="Rotate key" danger keepOpen isDisabled={locked} isPending={rotate.isPending} onConfirm={() => rotate.mutate(undefined, { onSuccess: () => setConfirming(false) })} onClose={() => setConfirming(false)}>
          <div className="kp-stack-3">
            <p className="kp-muted">Computers already linked with the old key keep working. The old key can no longer link a new one, and the new key shows once.</p>
            {rotate.isError ? (
              <Callout tone="danger" title="Not rotated">
                {reasonOf(rotate.error, 'the request failed.')}
              </Callout>
            ) : null}
          </div>
        </ConfirmDialog>
      ) : null}
    </span>
  );
}
