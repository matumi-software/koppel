import { cleanup } from '@testing-library/react';
import { afterEach, beforeEach, vi } from 'vitest';
import { FakeEventSource } from './src/test/fake-event-source';

// Testing Library only cleans up on its own when test globals are enabled.
afterEach(cleanup);

// No test opens a real stream: the live link talks to a fake one a test drives.
beforeEach(() => {
  FakeEventSource.reset();
  vi.stubGlobal('EventSource', FakeEventSource);
});

// jsdom has no scrolling; the page frame scrolls its side panel to the top.
Element.prototype.scrollTo = () => {};

// jsdom has no CSS global; react-aria's modal calls CSS.escape when it looks up a meta tag.
// Escaping every character outside [A-Za-z0-9_-] is valid CSS and enough for the plain names it passes.
if (typeof globalThis.CSS === 'undefined') {
  Object.defineProperty(globalThis, 'CSS', {
    configurable: true,
    value: { escape: (value: string) => value.replace(/[^\w-]/g, (c) => '\\' + c) },
  });
}
