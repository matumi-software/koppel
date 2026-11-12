import { Component, type ErrorInfo, type ReactNode } from 'react';
import { Button } from './Button';
import { Card, Row, Stack } from './Layout';
import { Note } from './Text';

export interface ErrorBoundaryProps {
  /** What to offer besides Reload, such as Sign out, so a page that cannot draw does not trap the person. */
  actions?: ReactNode;
  children: ReactNode;
}

/**
 * Catches an error while rendering what is under it and shows a plain card in its place, with Reload
 * and the actions the caller gives it, rather than a blank page.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, { error?: Error }> {
  state: { error?: Error } = {};

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Koppel: the page could not be drawn.', error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="kp-firstrun">
        <main className="kp-firstrun__main">
          <div className="kp-firstrun__column">
            <Card label="Something went wrong">
              <Stack gap="sm">
                <h1 className="kp-type-title">Something went wrong</h1>
                <Note>Koppel could not draw this page: {this.state.error.message}</Note>
                <Note>Reloading usually fixes it. Your settings are safe: they live on the server.</Note>
              </Stack>
              <Row justify="end">
                {this.props.actions}
                <Button variant="primary" onPress={() => window.location.reload()}>
                  Reload
                </Button>
              </Row>
            </Card>
          </div>
        </main>
      </div>
    );
  }
}
