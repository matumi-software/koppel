import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// Testing Library only cleans up on its own when test globals are enabled.
afterEach(cleanup);

// jsdom has no scrolling; the page frame scrolls its side panel to the top.
Element.prototype.scrollTo = () => {};
