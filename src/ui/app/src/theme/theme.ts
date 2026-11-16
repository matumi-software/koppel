import { useSyncExternalStore } from 'react';
import type { ThemePreference } from '@heartwood/ui';

/**
 * What this browser chose for the look: light, dark or follow the system, and colour vision mode, which
 * adds Heartwood's `-safe` variants. It is a per viewer choice, so it lives in this browser's
 * localStorage and not on the server. `applyTheme` puts the result on the root element as
 * `data-theme`, which is all Heartwood's stylesheet reads.
 */
export interface Look {
  theme: ThemePreference;
  colourVision: boolean;
}

const storageKey = 'koppel.look';
const defaults: Look = { theme: 'system', colourVision: false };

const isPreference = (value: unknown): value is ThemePreference => value === 'system' || value === 'light' || value === 'dark';

function read(): Look {
  try {
    const parsed: unknown = JSON.parse(window.localStorage.getItem(storageKey) ?? 'null');
    if (parsed && typeof parsed === 'object') {
      const { theme, colourVision } = parsed as Record<string, unknown>;
      return { theme: isPreference(theme) ? theme : defaults.theme, colourVision: colourVision === true };
    }
  } catch {
    // Blocked storage or a damaged value: the default look.
  }
  return defaults;
}

const systemDark = () => window.matchMedia('(prefers-color-scheme: dark)').matches;

/** The value of `data-theme` for a look: `system` resolves to light or dark by the operating system. */
export const themeOf = ({ theme, colourVision }: Look, prefersDark = systemDark()) =>
  `${theme === 'system' ? (prefersDark ? 'dark' : 'light') : theme}${colourVision ? '-safe' : ''}`;

let current = read();
const listeners = new Set<() => void>();

export function applyTheme(look: Look = current) {
  document.documentElement.dataset.theme = themeOf(look);
}

/** Applies the stored look before the first render, and follows the system while the choice is `system`. */
export function startTheme() {
  applyTheme();
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyTheme());
}

function change(next: Look) {
  current = next;
  try {
    window.localStorage.setItem(storageKey, JSON.stringify(next));
  } catch {
    // Private windows: the look lasts until the page closes.
  }
  applyTheme();
  listeners.forEach((listener) => listener());
}

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => void listeners.delete(listener);
};

/** The look and how to change it, for the Appearance tile of Settings (`SettingsPage`'s `appearance`). */
export function useLook() {
  const look = useSyncExternalStore(subscribe, () => current);
  return {
    ...look,
    setTheme: (theme: ThemePreference) => change({ ...current, theme }),
    setColourVision: (colourVision: boolean) => change({ ...current, colourVision }),
  };
}
