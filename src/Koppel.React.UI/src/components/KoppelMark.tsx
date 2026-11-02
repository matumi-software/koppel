export interface KoppelMarkProps {
  size?: number;
  /** Accessible name. Leave out when the word "Koppel" sits next to the mark. */
  title?: string;
}

/**
 * The Koppel mark: a K whose arms end in a circle (Shared) and a square (Attached).
 * Drawn in the accent token, so it follows the theme. Same geometry as
 * resources/design-system/assets/Logos/koppel-mark.svg.
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
      style={{ fill: 'var(--accent)', stroke: 'var(--accent)', flex: 'none' }}
    >
      <rect x="9" y="8" width="11" height="48" rx="2" stroke="none" />
      <path d="M20 32 L41 15" strokeWidth="9" fill="none" />
      <path d="M20 32 L40 49" strokeWidth="9" fill="none" />
      <circle cx="46" cy="12" r="8" stroke="none" />
      <rect x="38" y="42" width="16" height="16" rx="2" stroke="none" />
    </svg>
  );
}
