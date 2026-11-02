import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';

export type Page = 'overview' | 'devices' | 'clients' | 'attach' | 'machine' | 'connection';

const pages: readonly Page[] = ['overview', 'devices', 'clients', 'attach', 'machine', 'connection'];

const fromHash = (): Page => {
  const hash = window.location.hash.slice(1);
  return (pages as readonly string[]).includes(hash) ? (hash as Page) : 'overview';
};

/** The open page, kept in the address (#devices) so a phone can bookmark a page to check up on. */
export function usePageFromHash(): [Page, (page: Page) => void] {
  const [page, setPage] = useState<Page>(fromHash);
  useEffect(() => {
    const onHash = () => setPage(fromHash());
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  }, []);
  const go = (next: Page) => {
    window.location.hash = next;
    setPage(next);
  };
  return [page, go];
}

export interface FrameApi {
  page: Page;
  go: (page: Page) => void;
  nav: ReactNode;
  navFoot: ReactNode;
  navFootState: 'live' | 'reconnecting' | 'down';
  status: ReactNode;
  isMenuOpen: boolean;
  setMenuOpen: (open: boolean) => void;
  /** True while the UI's own link to its server is not live, so nothing may send a change. */
  locked: boolean;
}

export const FrameContext = createContext<FrameApi | null>(null);

export function useFrame(): FrameApi {
  const value = useContext(FrameContext);
  if (!value) throw new Error('useFrame must be used inside the app frame.');
  return value;
}
