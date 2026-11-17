import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { LogOut, RefreshCw, User } from 'lucide-react';
import { AppShell, Button, Callout, Popover, toast, StatusDot } from '@heartwood/ui';
import { brand } from '@/brand';
import { useAuth } from '@/auth/auth-context';
import { reasonOf } from '@/errors';
import { useLive } from '@/live/live-context';
import { useAnnounce } from '@/shared/announce-context';
import { useMedia } from '@/shared/use-media';
import { FrameContext, FrameSlotsContext, goTo, useHasNavigated } from './frame-context';
import { isPage, navGroups, places, type NavCounts, type Page } from './places';

/**
 * The strip's one piece of app-wide state: whether Koppel's own link to its service is live. When it
 * is not, nothing can be saved, and the button says why and offers to try again.
 */
function LinkState({ short }: { short?: boolean }) {
  const { state, retry } = useLive();
  if (state === 'live') return short ? <StatusDot tone="success" label="Live" /> : <StatusDot tone="success">Live</StatusDot>;
  const word = state === 'down' ? 'Not connected' : 'Reconnecting';
  return (
    <Popover
      placement="bottom end"
      title="Changes are paused"
      trigger={
        <Button variant="ghost" aria-label={short ? word : undefined}>
          {short ? <StatusDot tone="pending" label={word} /> : <StatusDot tone="pending">{word}</StatusDot>}
        </Button>
      }
      footer={
        <Button variant="secondary" icon={RefreshCw} isPending={state === 'reconnecting'} onPress={retry}>
          Try again
        </Button>
      }
    >
      Koppel lost its link to the service and keeps trying. If it stays down, check that the Koppel service is running.
    </Popover>
  );
}

/** On the page, where it can be seen: what the strip's label only hints at, with the way out in plain view. */
function LinkNotice() {
  const { state, retry } = useLive();
  return (
    <div className="kp-link-notice">
      <Callout
        tone="warning"
        title={state === 'down' ? 'Not connected to the Koppel service' : 'Reconnecting to the Koppel service'}
        actions={
          <Button variant="secondary" icon={RefreshCw} isPending={state === 'reconnecting'} onPress={retry}>
            Try again
          </Button>
        }
      >
        Changes are paused. If it stays down, check that the Koppel service is running.
      </Callout>
    </div>
  );
}

export interface KoppelShellProps {
  page: Page;
  /** The role, which decides the nav. Undefined until the settings load. */
  role: string | undefined;
  /** What the nav badges: only what asks for action. */
  counts?: NavCounts;
  children: ReactNode;
}

/**
 * The window every page of the app sits in: Heartwood's AppShell with the role's navigation, the
 * link state in the strip and the account menu in the foot. Pages reach it through three small
 * seams: `useFrame` (locked, narrow), and `useFlush` and `useInspector` (frame-context.ts).
 * Pages are passed as `children` and never re-render because of the shell's own state.
 */
export function KoppelShell({ page, role, counts, children }: KoppelShellProps) {
  const { state } = useLive();
  const { signOut } = useAuth();
  const announce = useAnnounce();
  const narrow = useMedia('(max-width: 799px)');
  const hasNavigated = useHasNavigated();
  const [flush, setFlush] = useState(false);
  const [inspector, setInspector] = useState<ReactNode>(null);
  const locked = state !== 'live';

  const frame = useMemo(() => ({ locked, narrow, hasNavigated }), [locked, narrow, hasNavigated]);
  const slots = useMemo(() => ({ setFlush, setInspector }), []);
  const groups = useMemo(() => navGroups(role, counts), [role, counts]);

  // A page that opens because the person went to it takes focus on its heading, so a keyboard or a
  // screen reader starts at the new page and not in the navigation; the first page of a visit leaves focus alone.
  const opened = useRef(page);
  useEffect(() => {
    if (opened.current === page) return;
    opened.current = page;
    const heading = document.querySelector<HTMLElement>('.hw-app-content h1');
    if (!heading) return;
    heading.tabIndex = -1;
    heading.focus({ preventScroll: true });
  }, [page]);

  const account = {
    id: 'account',
    label: 'admin',
    icon: User,
    menu: [
      {
        id: 'sign-out',
        label: 'Sign out',
        icon: LogOut,
        onAction: () => {
          signOut().catch((error: unknown) => {
            const reason = reasonOf(error, 'the request failed.');
            announce(`Signing out failed: ${reason}`);
            toast({ title: 'Could not sign out', text: reason, tone: 'danger' });
          });
        },
      },
    ],
  };

  return (
    <FrameContext.Provider value={frame}>
      <FrameSlotsContext.Provider value={slots}>
        <title>{`${places[page].label} - Koppel`}</title>
        <AppShell
          platform="web"
          brand={brand}
          nav={groups}
          footerNav={[account]}
          current={page}
          onNavigate={(id) => isPage(id) && goTo(id)}
          storageKey="koppel.sidebar"
          className={narrow ? 'kp-narrow' : undefined}
          toolbarEnd={<LinkState />}
          phoneEnd={<LinkState short />}
          flush={flush}
          inspector={inspector}
        >
          {locked ? <LinkNotice /> : null}
          {children}
        </AppShell>
      </FrameSlotsContext.Provider>
    </FrameContext.Provider>
  );
}
