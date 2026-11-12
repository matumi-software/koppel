import { useLayoutEffect, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { SideInfo } from '@/shared/side-info';
import { useFrame } from './frame-context';

export interface FrameProps {
  /** The page's heading. */
  title: string;
  lede?: ReactNode;
  /** What the side panel shows for this page, such as the selected device's details. Without it, facts and activity. */
  aside?: ReactNode;
  /** Changes when the aside shows something else, such as another device, so it starts at the top. */
  asideKey?: string;
  children: ReactNode;
}

/**
 * One page inside the app shell. The shell and its side panel stay put across pages; the page only
 * fills the main column and hands its aside content to the panel.
 */
export function Frame({ title, lede, aside, asideKey, children }: FrameProps) {
  const { asideSlot } = useFrame();

  useLayoutEffect(() => {
    asideSlot?.parentElement?.scrollTo({ top: 0 });
  }, [asideSlot, asideKey]);

  return (
    <>
      <div className="kp-page-head">
        <h1>{title}</h1>
        {lede && <p>{lede}</p>}
      </div>
      {children}
      {asideSlot && createPortal(aside ?? <SideInfo />, asideSlot)}
    </>
  );
}
