const key = 'koppel.aside-width';

/** The side panel width the user left it at in this browser, if any. */
export function storedAsideWidth(): number | undefined {
  try {
    const value = Number(window.localStorage.getItem(key));
    return value > 0 ? value : undefined;
  } catch {
    return undefined;
  }
}

export function storeAsideWidth(width: number) {
  try {
    window.localStorage.setItem(key, String(width));
  } catch {
    // Private windows and blocked storage: the width just resets on reload.
  }
}
