import type { ReactNode } from 'react';
import { KoppelMark } from '@koppel/react-ui';

/** The plain page the sign-in and password steps sit on: the Koppel mark, then one column. */
export function AuthFrame({ children }: { children: ReactNode }) {
  return (
    <div className="kp-shell" style={{ flexDirection: 'column' }}>
      <header className="kp-row" style={{ padding: '14px 24px', borderBottom: '1px solid var(--border-default)', background: 'var(--bg-surface)' }}>
        <KoppelMark size={26} />
        <span className="kp-type-heading">koppel</span>
      </header>
      <main style={{ display: 'flex', justifyContent: 'center', padding: '40px 16px' }}>
        <div style={{ width: '100%', maxWidth: 460, display: 'flex', flexDirection: 'column', gap: 24 }}>{children}</div>
      </main>
    </div>
  );
}
