import { useRef, useState } from 'react';
import { Button } from './Button';
import { copyText, selectText } from './clipboard';

export interface SecretValueProps {
  /** Fetches the secret when someone reveals or copies it. Nothing is fetched before that. */
  loadSecret: () => Promise<string>;
  /** Shown in place of the controls when there is no secret to show, with the reason. */
  unavailable?: string;
}

/**
 * A masked secret with Reveal and Copy, like Timebase Pulse's: the value is fetched on demand, once,
 * and kept in the component so Reveal then Copy does not ask twice.
 */
export function SecretValue({ loadSecret, unavailable }: SecretValueProps) {
  const [value, setValue] = useState<string>();
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState<'reveal' | 'copy'>();
  const [error, setError] = useState<string>();
  const code = useRef<HTMLElement>(null);

  if (unavailable) {
    return <p className="kp-field__help">{unavailable}</p>;
  }

  const resolve = async () => {
    if (value) return value;
    const fetched = await loadSecret();
    setValue(fetched);
    return fetched;
  };

  const run = async (action: 'reveal' | 'copy') => {
    setBusy(action);
    setError(undefined);
    try {
      const secret = await resolve();
      if (action === 'reveal') {
        setRevealed((shown) => !shown);
      } else {
        try {
          await copyText(secret);
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        } catch (e) {
          // Show the secret and select it, so Ctrl+C works.
          setRevealed(true);
          requestAnimationFrame(() => selectText(code.current));
          throw e;
        }
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not load the secret.');
    } finally {
      setBusy(undefined);
    }
  };

  return (
    <div className="kp-secret">
      {/* A label on <code> is not valid ARIA, and inside it the label would be copied with the secret. */}
      <span className="kp-sr-only">{revealed ? 'Client secret' : 'Client secret, hidden'}</span>
      <code ref={code} className="kp-id kp-secret__value">
        {revealed && value ? value : '••••••••••••••••••••'}
      </code>
      <div className="kp-row">
        <Button variant="ghost" isSending={busy === 'reveal'} onPress={() => void run('reveal')}>
          {revealed ? 'Hide' : 'Reveal'}
        </Button>
        <Button variant="ghost" isSending={busy === 'copy'} onPress={() => void run('copy')}>
          {copied ? 'Copied' : 'Copy'}
        </Button>
      </div>
      {error && (
        <span className="kp-error kp-type-caption" role="status">
          {error}
        </span>
      )}
    </div>
  );
}

/** Text with a Copy button, for IDs and fingerprints. */
export function CopyValue({ value, wrap }: { value: string; wrap?: boolean }) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string>();
  const code = useRef<HTMLElement>(null);

  const copy = () => {
    setError(undefined);
    copyText(value).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      (e: unknown) => {
        setCopied(false);
        setError(e instanceof Error ? e.message : 'Could not copy.');
        selectText(code.current);
      },
    );
  };

  return (
    <div style={{ display: 'grid', gap: 4 }}>
      <div className="kp-row" style={{ gap: 4 }}>
        <code ref={code} className="kp-id kp-secret__value" data-wrap={wrap ? 'true' : undefined}>
          {value}
        </code>
        <Button variant="ghost" onPress={copy}>
          {copied ? 'Copied' : 'Copy'}
        </Button>
      </div>
      {error && (
        <span className="kp-error kp-type-caption" role="status">
          {error}
        </span>
      )}
    </div>
  );
}
