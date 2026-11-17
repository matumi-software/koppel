import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';

// The service's answers, stood in by the test (the same JSON the unit tests and the Rust wire test share), so a
// browser test runs with no Koppel. Shared by the specs in this folder.
export const wire = (name: string) => JSON.parse(readFileSync(new URL(`../src/test/wire/${name}.json`, import.meta.url), 'utf8'));

export const signedOut = { ...wire('auth-state'), setupTokenRequired: false };

export async function answerSignedOut(page: Page, state: object) {
  await page.route('**/api/v1/auth/state', (route) => route.fulfill({ json: state }));
}

export interface ServeOptions {
  /** Fields laid over the settings fixture. */
  settings?: object;
  /** Fields laid over the system info fixture. */
  info?: object;
  /** Whole answers, by path, laid over the fixtures' (and pushed again where a push carries them). */
  answers?: Record<string, unknown>;
  /** `up` (the default) opens the live link at once. `reconnecting` never opens it. `down` fails it until the app calls it down. */
  link?: 'up' | 'reconnecting' | 'down';
}

/** A server whose screens the Overview needs, answered from the fixtures. */
export async function serve(page: Page, role: 'server' | 'client' | 'both', options: ServeOptions = {}) {
  const settings = { ...wire('settings'), role, setupComplete: true, ...options.settings };
  const servers = options.answers?.['/api/v1/client/servers'] ?? wire('servers');
  const answers: Record<string, unknown> = {
    '/api/v1/auth/state': { ...signedOut, signedIn: true, passwordSet: true, setupOpen: false },
    '/api/v1/settings': settings,
    '/api/v1/system/info': { ...wire('system-info'), role, ...options.info },
    '/api/v1/system/checks': wire('system-checks').map((c: object) => ({ ...c, ok: true })),
    '/api/v1/activity': [wire('activity-event')],
    '/api/v1/server/clients': wire('clients'),
    '/api/v1/server/devices': wire('server-devices'),
    '/api/v1/server/enrollment': wire('enrollment-view'),
    '/api/v1/server/keys': wire('keys-view'),
    '/api/v1/server/identity': wire('server-identity'),
    '/api/v1/server/pairing': wire('pairing-overview'),
    '/api/v1/client/servers': servers,
    '/api/v1/client/devices': wire('client-devices'),
    '/api/v1/client/discovery': wire('discovered-servers'),
    ...options.answers,
  };
  const push: [string, unknown][] = [
    ['SettingsChanged', settings],
    ...(role !== 'client'
      ? ([['ClientsChanged', answers['/api/v1/server/clients']], ['ServerKeysChanged', answers['/api/v1/server/keys']], ['PairingRequestsChanged', answers['/api/v1/server/pairing']], ['EnrollmentChanged', answers['/api/v1/server/enrollment']], ['DevicesChanged', answers['/api/v1/server/devices']]] as [string, unknown][])
      : []),
    ...(role !== 'server'
      ? ([['ServersChanged', servers], ['ServersDiscovered', answers['/api/v1/client/discovery']], ['ClientDevicesChanged', answers['/api/v1/client/devices']]] as [string, unknown][])
      : []),
  ];
  // The live push is a stand-in for EventSource that opens at once, sends the snapshot and stays open, so the
  // app is live and its controls work, as with a real server. A spec can send a later event with
  // `window.__push(name, data)`.
  await page.addInitScript(([events, link]: [[string, unknown][], string]) => {
    const sources: FakeEventSource[] = [];
    class FakeEventSource {
      readyState = 0;
      onopen: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      private listeners: Record<string, ((event: { data: string }) => void)[]> = {};
      url: string;
      constructor(url: string) {
        this.url = url;
        sources.push(this);
        if (link === 'reconnecting') return;
        if (link === 'down') {
          setTimeout(() => this.onerror?.(new Event('error')), 0);
          return;
        }
        setTimeout(() => {
          this.readyState = 1;
          this.onopen?.(new Event('open'));
          for (const [name, data] of events) this.emit(name, data);
        }, 0);
      }
      addEventListener(name: string, listener: (event: { data: string }) => void) {
        (this.listeners[name] ??= []).push(listener);
      }
      removeEventListener() {}
      close() {
        this.readyState = 2;
      }
      emit(name: string, data: unknown) {
        for (const listener of this.listeners[name] ?? []) listener({ data: JSON.stringify(data) });
      }
    }
    (window as unknown as { EventSource: unknown }).EventSource = FakeEventSource;
    (window as unknown as { __push: (name: string, data: unknown) => void }).__push = (name, data) => {
      for (const source of sources) source.emit(name, data);
    };
  }, [push, options.link ?? 'up'] as [[string, unknown][], string]);
  await page.route('**/api/v1/**', (route) => {
    const request = route.request();
    const { pathname } = new URL(request.url());
    if (request.method() !== 'GET') {
      // A change is accepted; the page waits for the push, as it does with a real server.
      return route.fulfill({ status: 202, body: '' });
    }
    return route.fulfill({ json: answers[pathname] ?? [] });
  });
}
