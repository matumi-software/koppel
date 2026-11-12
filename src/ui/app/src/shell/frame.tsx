import { useCallback, useLayoutEffect, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { PageHead } from '@koppel/react-ui';
import { SideInfo } from '@/shared/side-info';
import { hasNavigated, useFrame } from './frame-context';

export interface FrameProps {
  /** The page's heading. It is also the browser tab's title. */
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
 * fills the main column and hands its aside content to the panel. A page that opens because the
 * person went to it takes focus on its heading, so a keyboard or a screen reader starts at the new
 * page and not in the navigation it came from; the first page of a visit leaves focus alone.
 */
export function Frame({ title, lede, aside, asideKey, children }: FrameProps) {
  const { asideSlot } = useFrame();
  // A stable callback, so it runs when the heading mounts and not on every render after it.
  const focusOnOpen = useCallback((element: HTMLHeadingElement | null) => {
    if (element && hasNavigated()) element.focus({ preventScroll: true });
  }, []);
  useLayoutEffect(() => {
    asideSlot?.scrollTo({ top: 0 });
  }, [asideSlot, asideKey]);

  return (
    <>
      <title>{`${title} - Koppel`}</title>
      <PageHead title={title} lede={lede} headingRef={focusOnOpen} />
      {children}
      {asideSlot && createPortal(aside ?? <SideInfo />, asideSlot)}
    </>
  );
}
