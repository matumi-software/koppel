/** The six digits a client shows while it waits to pair, as two groups of three. */
export function PairingCode({ code }: { code: string }) {
  const digits = code.replace(/\D/g, '');
  return (
    <div className="kp-paircode">
      {/* Screen readers get the digits one by one; the visible groups are hidden from them. */}
      <span className="kp-sr-only">Pairing code {digits.split('').join(' ')}</span>
      <span className="kp-paircode__digits" aria-hidden="true">
        <span>{digits.slice(0, 3)}</span>
        <span>{digits.slice(3)}</span>
      </span>
    </div>
  );
}
