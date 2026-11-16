import { CircleAlert, RefreshCw } from 'lucide-react';
import { Button, EmptyState } from '@matumi/ui';

/**
 * Koppel does not answer at all: nothing is loaded, so there is nothing to lock. One way on, Try
 * again, which asks for what was missing.
 */
export function CouldNotReach({ isRetrying, onRetry }: { isRetrying?: boolean; onRetry: () => void }) {
  return (
    <div className="kp-fill">
      <EmptyState
        tone="danger"
        icon={CircleAlert}
        title="Could not reach Koppel"
        actions={
          <Button variant="secondary" icon={RefreshCw} isPending={isRetrying} onPress={onRetry}>
            Try again
          </Button>
        }
      >
        Check that the Koppel service is running on this computer, then try again.
      </EmptyState>
    </div>
  );
}
