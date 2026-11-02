import { useState } from 'react';
import { Button } from '@koppel/react-ui';

/** A ghost Button that copies text and says so for a moment. */
export function CopyButton({ text, label = 'Copy' }: { text: string; label?: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      () => setCopied(false),
    );
  };
  return (
    <Button variant="ghost" onPress={copy}>
      {copied ? 'Copied' : label}
    </Button>
  );
}
