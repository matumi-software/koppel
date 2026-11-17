import { createContext, useContext, useEffect, useLayoutEffect, useState, useSyncExternalStore, type ReactNode } from 'react';
import { isPage, type Page } from './places';

const subscribe = (onChange: () => void) => {
  window.addEventListener('hashchange', onChange);
  return () => window.removeEventListener('hashchange', onChange);
};

const hash = () => window.location.hash;

/**
 * The open page, kept in the address (#remote) so a phone can bookmark a page to check up on. The
 * address is the one place it lives: `goTo` changes it and this reads it back, so nothing is set twice.
 * With no page in the address (or one that does not exist) it is `fallback`.
 */
export function usePage(fallback: Page = 'overview'): Page {
  const value = useSyncExternalStore(subscribe, hash).slice(1);
  return isPage(value) ? value : fallback;
}

/**
 * True once the person has gone to another page since this was first used, as opposed to the page
 * the visit opened on.
 */
export function useHasNavigated(): boolean {
  const [navigated, setNavigated] = useState(false);
  useEffect(() => {
    const onChange = () => setNavigated(true);
    window.addEventListener('hashchange', onChange);
    return () => window.removeEventListener('hashchange', onChange);
  }, []);
  return navigated;
}

/** Opens a page. */
export const goTo = (page: Page) => {
  window.location.hash = page;
};

export interface FrameApi {
  /** True once the person has gone to another page, so a page that opens now takes focus on its heading. */
  hasNavigated: boolean;
  /** True while the UI's own link to its server is not live, so nothing may send a change. */
  locked: boolean;
  /** True while the settings panel is a sheet over the page (Heartwood's own condition, `useInspectorSheet`): a ListDetail then shows one pane at a time and the tables drop columns. */
  narrow: boolean;
}

export const FrameContext = createContext<FrameApi | null>(null);

/** What the shell tells every page. Pages read `locked` to disable what sends a change. */
export function useFrame(): FrameApi {
  const value = useContext(FrameContext);
  if (!value) throw new Error('useFrame must be used inside the app frame.');
  return value;
}

/** What a page hands up to the shell. The shell keeps it in state; nothing here makes a page re-render. */
export interface FrameSlots {
  setFlush: (flush: boolean) => void;
  setInspector: (inspector: ReactNode) => void;
}

export const FrameSlotsContext = createContext<FrameSlots | null>(null);

function useSlots(): FrameSlots {
  const value = useContext(FrameSlotsContext);
  if (!value) throw new Error('useFlush and useInspector must be used inside the app frame.');
  return value;
}

/**
 * A page whose content runs edge to edge in the panel, as a ListDetail does: call it with true while
 * that is so. The shell's AppShell `flush` follows, and goes back when the page leaves.
 */
export function useFlush(flush: boolean) {
  const { setFlush } = useSlots();
  useLayoutEffect(() => {
    setFlush(flush);
    return () => setFlush(false);
  }, [flush, setFlush]);
}

/**
 * A page with an Inspector (Connection): pass the `<Inspector>` element, or null while there is none.
 * It is handed to AppShell's `inspector` on every render of the page, and removed when the page leaves.
 */
export function useInspector(inspector: ReactNode) {
  const { setInspector } = useSlots();
  useLayoutEffect(() => {
    setInspector(inspector);
  });
  useLayoutEffect(() => () => setInspector(null), [setInspector]);
}
