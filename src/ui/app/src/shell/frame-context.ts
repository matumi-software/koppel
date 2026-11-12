import { createContext, useContext, useEffect, useState, useSyncExternalStore } from 'react';

export type Page = 'overview' | 'devices' | 'clients' | 'attach' | 'configuration';

/** The phone top bar's title for each page. A page's own heading may say more, such as which server. */
export const pageTitles: Record<Page, string> = {
  overview: 'Overview',
  devices: 'Devices',
  clients: 'Clients',
  attach: 'Devices',
  configuration: 'Configuration',
};

const isPage = (value: string): value is Page => Object.hasOwn(pageTitles, value);

const subscribe = (onChange: () => void) => {
  window.addEventListener('hashchange', onChange);
  return () => window.removeEventListener('hashchange', onChange);
};

const hash = () => window.location.hash;

/**
 * The open page, kept in the address (#devices) so a phone can bookmark a page to check up on. The
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
  /** The one side panel. A page renders its content there through Frame's aside. */
  asideSlot: HTMLElement | null;
}

export const FrameContext = createContext<FrameApi | null>(null);

export function useFrame(): FrameApi {
  const value = useContext(FrameContext);
  if (!value) throw new Error('useFrame must be used inside the app frame.');
  return value;
}
