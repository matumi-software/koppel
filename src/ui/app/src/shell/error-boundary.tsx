import { Component, type ErrorInfo, type ReactNode } from 'react';
import { CircleAlert, RefreshCw } from 'lucide-react';
import { Button, EmptyState } from '@matumi/ui';

interface Props {
  /** More buttons beside Reload, such as Sign out. */
  actions?: ReactNode;
  children: ReactNode;
}

/**
 * A render error must not leave a blank page: it shows what happened with Reload, and under the app
 * Sign out too. Error boundaries are class components, which is why this one is not a hook.
 */
export class ErrorBoundary extends Component<Props, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Koppel: the page failed to render.', error, info.componentStack);
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="kp-fill">
        <EmptyState
          tone="danger"
          icon={CircleAlert}
          title="Something went wrong"
          actions={
            <>
              <Button variant="secondary" icon={RefreshCw} onPress={() => window.location.reload()}>
                Reload
              </Button>
              {this.props.actions}
            </>
          }
        >
          Koppel could not draw this page. Reloading usually fixes it.
        </EmptyState>
      </div>
    );
  }
}
