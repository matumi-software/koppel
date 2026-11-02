import type { ReactNode } from 'react';
import { AppShell } from '@koppel/react-ui';
import { useFrame } from './frame-context';

export interface FrameProps {
  /** The page title, also shown in the phone top bar. */
  title: string;
  lede?: ReactNode;
  /** The right column: activity, or a device's details. */
  aside?: ReactNode;
  asideWide?: boolean;
  children: ReactNode;
}

/** One page inside the app shell. */
export function Frame({ title, lede, aside, asideWide, children }: FrameProps) {
  const frame = useFrame();
  return (
    <AppShell
      nav={frame.nav}
      navFoot={frame.navFoot}
      navFootState={frame.navFootState}
      aside={aside}
      asideWide={asideWide}
      title={title}
      status={frame.status}
      isMenuOpen={frame.isMenuOpen}
      onMenuOpenChange={frame.setMenuOpen}
    >
      <div className="kp-page-head">
        <h1>{title}</h1>
        {lede && <p>{lede}</p>}
      </div>
      {children}
    </AppShell>
  );
}
