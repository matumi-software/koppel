import type { ReactNode } from 'react';
import { Button, Dialog, Link, Modal, ModalOverlay } from 'react-aria-components';
import { KoppelMark } from './KoppelMark';

export interface AppShellProps {
  /** Sidebar content: NavGroups and the link status at the bottom. Also shown in the phone menu. */
  nav: ReactNode;
  /** Shown at the bottom of the sidebar, such as LinkBanner. */
  navFoot?: ReactNode;
  navFootState?: 'live' | 'reconnecting' | 'down';
  /** Right column: activity, or a device's details. Folds below the page at phone width. */
  aside?: ReactNode;
  /** A wider right column, for details. */
  asideWide?: boolean;
  /** The phone top bar's title, such as the open page. */
  title: string;
  /** Small status for the phone top bar, such as the live dot. */
  status?: ReactNode;
  isMenuOpen: boolean;
  onMenuOpenChange: (open: boolean) => void;
  children: ReactNode;
}

/**
 * The app frame: sidebar navigation, the page, and an aside. At phone width the sidebar moves into
 * a menu behind a top-bar button and the aside folds below the page, so nothing scrolls sideways.
 */
export function AppShell({ nav, navFoot, navFootState, aside, asideWide, title, status, isMenuOpen, onMenuOpenChange, children }: AppShellProps) {
  const brand = (
    <div className="kp-brand">
      <KoppelMark size={26} />
      <span>Koppel</span>
    </div>
  );
  const foot = navFoot && (
    <div className="kp-shell__foot" data-state={navFootState}>
      {navFoot}
    </div>
  );

  return (
    <div className="kp-shell">
      <header className="kp-shell__top">
        <Button className="kp-button" data-variant="ghost" aria-label="Open menu" onPress={() => onMenuOpenChange(true)} style={{ minWidth: 44, minHeight: 44 }}>
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" aria-hidden="true">
            <path d="M4 6h16" />
            <path d="M4 12h16" />
            <path d="M4 18h16" />
          </svg>
        </Button>
        <KoppelMark size={22} />
        <span className="kp-type-heading" style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {title}
        </span>
        {status}
      </header>

      <aside className="kp-shell__side" aria-label="Navigation">
        {brand}
        {nav}
        {foot}
      </aside>

      <ModalOverlay className="kp-overlay" isOpen={isMenuOpen} onOpenChange={onMenuOpenChange} isDismissable>
        <Modal className="kp-menu">
          <Dialog aria-label="Menu" style={{ display: 'contents' }}>
            {brand}
            {nav}
            {foot}
          </Dialog>
        </Modal>
      </ModalOverlay>

      <main className="kp-shell__main">{children}</main>

      {aside && (
        <aside className="kp-shell__aside" data-wide={asideWide ? 'true' : undefined}>
          {aside}
        </aside>
      )}
    </div>
  );
}

export function NavGroup({ label, children }: { label?: string; children: ReactNode }) {
  return (
    <nav className="kp-nav" aria-label={label ?? 'Main'}>
      {label && <span className="kp-nav__label">{label}</span>}
      {children}
    </nav>
  );
}

export interface NavItemProps {
  isCurrent?: boolean;
  count?: number | string;
  countTone?: 'pending';
  onPress: () => void;
  children: ReactNode;
}

/** One page in the navigation. A react-aria Link, marked with aria-current when open. */
export function NavItem({ isCurrent, count, countTone, onPress, children }: NavItemProps) {
  return (
    <Link className="kp-nav__item" aria-current={isCurrent ? 'page' : undefined} onPress={onPress}>
      {children}
      {count !== undefined && (
        <span className="kp-nav__count" data-tone={countTone}>
          {count}
        </span>
      )}
    </Link>
  );
}
