import { act } from '@testing-library/react';

/**
 * A stand-in for the browser's EventSource that a test drives by hand: it answers at once unless the
 * server is away, a test fails it, and a test pushes events through it. vitest.setup.ts installs it for
 * every test and resets it between them.
 */
export class FakeEventSource {
  static instances: FakeEventSource[] = [];
  /** False while the server is away: a new stream fails instead of opening. */
  static answering = true;

  readonly url: string;
  opened = false;
  closed = false;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  private readonly listeners = new Map<string, ((event: MessageEvent<string>) => void)[]>();

  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
    queueMicrotask(() => {
      if (this.closed) return;
      act(() => {
        if (FakeEventSource.answering) {
          this.opened = true;
          this.onopen?.();
        } else {
          this.fail();
        }
      });
    });
  }

  addEventListener(name: string, listener: (event: MessageEvent<string>) => void) {
    this.listeners.set(name, [...(this.listeners.get(name) ?? []), listener]);
  }

  close() {
    this.closed = true;
  }

  /** The stream broke, or the server ended it, as it does when the session ends. */
  fail() {
    this.onerror?.();
  }

  /** How many listeners the app registered for an event on this stream. */
  count(name: string) {
    return this.listeners.get(name)?.length ?? 0;
  }

  /** Delivers the raw text of one event, as the server sends it. */
  emitRaw(name: string, data: string) {
    for (const listener of this.listeners.get(name) ?? []) listener(new MessageEvent(name, { data }));
  }

  static reset() {
    FakeEventSource.instances.length = 0;
    FakeEventSource.answering = true;
  }

  /** The streams the app has open now. */
  static get live() {
    return FakeEventSource.instances.filter((s) => s.opened && !s.closed);
  }

  /** The stream the app is listening on now (the newest that is not closed). */
  static get current() {
    const open = FakeEventSource.instances.filter((s) => !s.closed);
    return open[open.length - 1];
  }

  /** Pushes one event to the app, as the server does: the name and the JSON of the value. */
  static emit(name: string, payload: unknown) {
    FakeEventSource.current.emitRaw(name, JSON.stringify(payload));
  }
}
