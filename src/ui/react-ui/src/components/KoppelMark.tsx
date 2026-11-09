export interface KoppelMarkProps {
  size?: number;
  /** Accessible name. Leave out when the wordmark "koppel" sits next to the mark. */
  title?: string;
}

/**
 * The Koppel mark: a K whose arms end in a circle and an arrow, after the USB trident.
 * The K is drawn in the text-primary token and the circle in the accent token, so it follows the
 * theme. Same geometry and colours as resources/design-system/assets/Logos/koppel-mark.svg.
 */
export function KoppelMark({ size = 24, title }: KoppelMarkProps) {
  return (
    <svg
      viewBox="0 0 64 64"
      width={size}
      height={size}
      role={title ? 'img' : undefined}
      aria-hidden={title ? undefined : true}
      aria-label={title}
      style={{ fill: 'var(--text-primary)', stroke: 'var(--text-primary)', flex: 'none' }}
    >
      <rect x="9" y="8" width="11" height="48" rx="2" stroke="none" />
      <path d="M20 32 L41 15" strokeWidth="9" fill="none" />
      <path d="M20 32 L41.7 47.8" strokeWidth="9" fill="none" />
      <path d="M53 56 L35.6 54.5 L46.2 39.9 Z" strokeWidth="3" strokeLinejoin="round" />
      <circle cx="46" cy="12" r="8" stroke="none" style={{ fill: 'var(--accent)' }} />
    </svg>
  );
}
