import { useRef, useState } from 'react';
import { useAnnounce } from './Announcer';
import { Button } from './Button';
import { copyText, selectText } from './clipboard';
import { Row } from './Layout';
import { Caption } from './Text';

export interface SecretValueProps {
  /** What the secret is, such as "Client secret": its name for screen readers. */
  label: string;
  /** Fetches the secret when someone reveals or copies it. Nothing is fetched before that. */
  loadSecret: () => Promise<string>;
  /** Shown in place of the controls when there is no secret to show, with the reason. */
  unavailable?: string;
}

/**
 * A masked secret with Reveal and Copy. The value is fetched on demand and held only while it is on
 * show: Hide drops it, and so does a copy made while it is masked, so a secret never sits in the
 * page longer than it was asked for.
 */
export function SecretValue({ label, loadSecret, unavailable }: SecretValueProps) {
  const [value, setValue] = useState<string>();
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState<'reveal' | 'copy'>();
  const [error, setError] = useState<string>();
  const code = useRef<HTMLElement>(null);
  const announce = useAnnounce();

  if (unavailable) {
    return <p className="kp-field__help">{unavailable}</p>;
  }

  const run = async (action: 'reveal' | 'copy') => {
    setBusy(action);
    setError(undefined);
    try {
      if (action === 'reveal' && revealed) {
        setRevealed(false);
        setValue(undefined);
        return;
      }
      const secret = value ?? (await loadSecret());
      if (action === 'reveal') {
        setValue(secret);
        setRevealed(true);
      } else {
        try {
          await copyText(secret);
          setCopied(true);
          announce('Copied');
          setTimeout(() => setCopied(false), 1500);
          if (!revealed) setValue(undefined);
        } catch (e) {
          // Show the secret and select it, so Ctrl+C works.
          setValue(secret);
          setRevealed(true);
          requestAnimationFrame(() => selectText(code.current));
          throw e;
        }
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : 'Could not load the secret.';
      setError(message);
      announce(message);
    } finally {
      setBusy(undefined);
    }
  };

  return (
    <div className="kp-secret">
      {/* A label on <code> is not valid ARIA, and inside it the label would be copied with the secret. */}
      <span className="kp-sr-only">{revealed ? label : `${label}, hidden`}</span>
      <code ref={code} className="kp-id kp-secret__value">
        {revealed && value ? value : '••••••••••••••••••••'}
      </code>
      <Row gap="xs">
        <Button variant="ghost" isSending={busy === 'reveal'} onPress={() => void run('reveal')}>
          {revealed ? 'Hide' : 'Reveal'}
        </Button>
        <Button variant="ghost" isSending={busy === 'copy'} onPress={() => void run('copy')}>
          {copied ? 'Copied' : 'Copy'}
        </Button>
      </Row>
      {error && <Caption tone="fault">{error}</Caption>}
    </div>
  );
}

/** Text with a Copy button, for IDs and fingerprints. */
export function CopyValue({ value, wrap }: { value: string; wrap?: boolean }) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string>();
  const code = useRef<HTMLElement>(null);
  const announce = useAnnounce();

  const copy = () => {
    setError(undefined);
    copyText(value).then(
      () => {
        setCopied(true);
        announce('Copied');
        setTimeout(() => setCopied(false), 1500);
      },
      (e: unknown) => {
        setCopied(false);
        const message = e instanceof Error ? e.message : 'Could not copy.';
        setError(message);
        announce(message);
        selectText(code.current);
      },
    );
  };

  return (
    <div className="kp-secret">
      <Row gap="xs">
        <code ref={code} className="kp-id kp-secret__value" data-wrap={wrap ? 'true' : undefined}>
          {value}
        </code>
        <Button variant="ghost" onPress={copy}>
          {copied ? 'Copied' : 'Copy'}
        </Button>
      </Row>
      {error && <Caption tone="fault">{error}</Caption>}
    </div>
  );
}
