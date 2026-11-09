/** Shown when nothing could copy. The person can still do it by hand. */
export const copyFailedMessage = 'Could not copy automatically. Select the text and press Ctrl+C.';

/** The old way: select the text in a hidden field and ask the browser to copy the selection. */
function copyBySelection(text: string): boolean {
  const field = document.createElement('textarea');
  field.value = text;
  field.setAttribute('readonly', '');
  field.setAttribute('aria-hidden', 'true');
  field.style.cssText = 'position:fixed;top:0;left:0;width:1px;height:1px;opacity:0;pointer-events:none';
  const focused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  document.body.appendChild(field);
  try {
    field.select();
    field.setSelectionRange(0, text.length);
    return typeof document.execCommand === 'function' && document.execCommand('copy');
  } finally {
    field.remove();
    focused?.focus();
  }
}

/**
 * Copies text. The clipboard API only exists on a secure context (HTTPS or localhost), and Koppel is
 * often opened over plain HTTP on the LAN, so this falls back to selecting the text and copying the
 * selection. Rejects with a message that is ready to show a person, never a raw TypeError.
 */
export async function copyText(text: string): Promise<void> {
  try {
    if (window.isSecureContext && navigator.clipboard) {
      await navigator.clipboard.writeText(text);
      return;
    }
  } catch {
    // Refused (permission, focus): try the fallback.
  }
  let copied = false;
  try {
    copied = copyBySelection(text);
  } catch {
    copied = false;
  }
  if (!copied) throw new Error(copyFailedMessage);
}

/** Selects an element's text, so Ctrl+C copies it when nothing else could. */
export function selectText(element: HTMLElement | null) {
  try {
    if (element) window.getSelection()?.selectAllChildren(element);
  } catch {
    // Nothing more to try.
  }
}
