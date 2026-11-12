import { useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent, type ReactNode, type Ref } from 'react';
import { Button, Dialog, Link, Modal, ModalOverlay } from 'react-aria-components';
import { KoppelMark } from './KoppelMark';

export interface AppShellProps {
  /** Sidebar content: NavGroups. Also shown in the phone menu. */
  nav: ReactNode;
  /** Shown at the bottom of the sidebar and the phone menu, such as StatusCard. */
  navFoot?: ReactNode;
  /**
   * Right column: activity, or the selected item's details. One panel for every page, whose width
   * the user sets by dragging its edge and its content never changes. Folds below the page at phone width.
   */
  aside?: ReactNode;
  /**
   * The aside's body, for a page that puts its own content in the side panel (a portal). Giving it
   * makes the panel exist even while `aside` is empty.
   */
  asideRef?: Ref<HTMLDivElement>;
  /** The aside's width in pixels when the shell first shows, such as the width the user left it at. */
  defaultAsideWidth?: number;
  /** Called with the new width when the user finishes resizing the aside. */
  onAsideWidthChange?: (width: number) => void;
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
export function AppShell({ nav, navFoot, aside, asideRef, defaultAsideWidth, onAsideWidthChange, title, status, isMenuOpen, onMenuOpenChange, children }: AppShellProps) {
  const [asideWidth, setAsideWidth] = useState(() => clampAside(defaultAsideWidth ?? asideDefault));
  const [isResizing, setResizing] = useState(false);
  const brand = (
    <div className="kp-brand">
      <KoppelMark size={26} />
      <span>koppel</span>
    </div>
  );
  const foot = navFoot && <div className="kp-shell__foot">{navFoot}</div>;
  const main = useRef<HTMLElement>(null);

  return (
    <div className="kp-shell" data-resizing={isResizing ? 'true' : undefined}>
      <a
        className="kp-skip"
        href="#main"
        onClick={(event) => {
          // A hash link would change the address, which is where the open page lives.
          event.preventDefault();
          main.current?.focus();
        }}
      >
        Skip to the page
      </a>
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

      <main ref={main} id="main" tabIndex={-1} className="kp-shell__main">
        {children}
      </main>

      {(aside || asideRef) && (
        <aside className="kp-shell__aside" aria-label="Details" style={{ '--kp-aside-width': `${asideWidth}px` } as CSSProperties}>
          <AsideResizer
            width={asideWidth}
            isResizing={isResizing}
            onResize={setAsideWidth}
            onResizingChange={setResizing}
            onDone={(width) => onAsideWidthChange?.(width)}
          />
          <div ref={asideRef} className="kp-shell__aside-body">
            {aside}
          </div>
        </aside>
      )}
    </div>
  );
}

const asideMin = 280;
const asideMax = 560;
const asideDefault = 340;
const asideStep = 16;
const clampAside = (width: number) => Math.round(Math.min(asideMax, Math.max(asideMin, width)));

interface AsideResizerProps {
  width: number;
  isResizing: boolean;
  onResize: (width: number) => void;
  onResizingChange: (isResizing: boolean) => void;
  onDone: (width: number) => void;
}

/**
 * The aside's left edge as a focusable separator. Dragging it left widens the aside; with focus,
 * the arrow keys move it in steps and Home and End jump to the narrowest and widest.
 */
function AsideResizer({ width, isResizing, onResize, onResizingChange, onDone }: AsideResizerProps) {
  const drag = useRef<{ x: number; width: number } | null>(null);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { x: e.clientX, width };
    onResizingChange(true);
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (drag.current) onResize(clampAside(drag.current.width + drag.current.x - e.clientX));
  };
  const onPointerEnd = () => {
    if (!drag.current) return;
    drag.current = null;
    onResizingChange(false);
    onDone(width);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const next =
      e.key === 'ArrowLeft' ? width + asideStep : e.key === 'ArrowRight' ? width - asideStep : e.key === 'Home' ? asideMin : e.key === 'End' ? asideMax : undefined;
    if (next === undefined) return;
    e.preventDefault();
    const clamped = clampAside(next);
    onResize(clamped);
    onDone(clamped);
  };

  return (
    <div
      className="kp-shell__resize"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the side panel"
      aria-valuenow={width}
      aria-valuemin={asideMin}
      aria-valuemax={asideMax}
      tabIndex={0}
      data-dragging={isResizing ? 'true' : undefined}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onKeyDown={onKeyDown}
    />
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
  /** Where the link goes, such as #devices, so it opens in a new tab and takes a middle click. */
  href: string;
  /** Also called when the link is followed, such as to close the phone menu. */
  onPress?: () => void;
  children: ReactNode;
}

/** One page in the navigation. A real link (react-aria Link), marked with aria-current when open. */
export function NavItem({ isCurrent, count, countTone, href, onPress, children }: NavItemProps) {
  return (
    <Link className="kp-nav__item" href={href} aria-current={isCurrent ? 'page' : undefined} onPress={onPress}>
      {children}
      {count !== undefined && (
        <span className="kp-nav__count" data-tone={countTone}>
          {count}
        </span>
      )}
    </Link>
  );
}
