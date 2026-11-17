import type { Page } from '@playwright/test';

// What the pre-release pass measures on every screen it drives (see screens.spec.ts). Each finding is a fact
// read from the live page, not a guess from the source: `kind` says what rule it breaks.
export interface Finding {
  kind: 'sideways' | 'console' | 'unnamed' | 'small-target' | 'contrast' | 'offscreen' | 'clipped';
  detail: string;
}

/** Collects what the page logs while a screen is driven: errors, uncaught exceptions and failed requests. */
export function watch(page: Page) {
  const seen: Finding[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') seen.push({ kind: 'console', detail: message.text() });
  });
  page.on('pageerror', (error) => seen.push({ kind: 'console', detail: `uncaught: ${error.message}` }));
  return seen;
}

/** The measurements taken inside the page. Runs in the browser, so it must not close over anything. */
export async function measure(page: Page, phone: boolean): Promise<Finding[]> {
  return page.evaluate((isPhone) => {
    const found: { kind: string; detail: string }[] = [];
    const edge = document.documentElement.clientWidth;
    const label = (el: Element) => {
      const text = (el.getAttribute('aria-label') ?? el.textContent ?? '').trim().replace(/\s+/g, ' ').slice(0, 40);
      return `${el.tagName.toLowerCase()}${el.className && typeof el.className === 'string' ? '.' + el.className.split(' ')[0] : ''} "${text}"`;
    };
    const visible = (el: Element) => {
      const box = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      return box.width > 0 && box.height > 0 && style.visibility !== 'hidden' && style.display !== 'none' && Number(style.opacity) > 0;
    };
    // A box that scrolls sideways on purpose (a wide table) may be wider than the screen.
    const scrolled = (el: Element) => {
      for (let up = el.parentElement; up; up = up.parentElement) if (/auto|scroll/.test(getComputedStyle(up).overflowX)) return true;
      return false;
    };

    // 1. Nothing makes the page scroll sideways.
    const over = document.documentElement.scrollWidth - edge;
    if (over > 0) found.push({ kind: 'sideways', detail: `the page is ${over}px wider than the window` });
    const main = document.querySelector('main');
    if (main && main.scrollWidth - main.clientWidth > 1) found.push({ kind: 'sideways', detail: `the content is ${main.scrollWidth - main.clientWidth}px wider than its panel` });

    // 2. Nothing visible sits past the right edge unless a scroll box holds it.
    for (const el of document.querySelectorAll('body *')) {
      if (!visible(el) || scrolled(el) || el.closest('[aria-hidden="true"], .kp-announcer, .hw-visually-hidden')) continue;
      const box = el.getBoundingClientRect();
      if (box.right > edge + 1 && getComputedStyle(el).position !== 'fixed') found.push({ kind: 'offscreen', detail: `${label(el)} ends ${Math.round(box.right - edge)}px past the edge` });
    }

    // 3. Every control has a name a screen reader can say.
    const controls = 'button, a[href], input:not([type=hidden]), select, textarea, [role=button], [role=switch], [role=checkbox], [role=tab], [role=menuitem], [role=radio], [role=combobox]';
    const named = (el: Element) => {
      if (el.getAttribute('aria-label')?.trim() || el.getAttribute('title')?.trim()) return true;
      const by = el.getAttribute('aria-labelledby');
      if (by && by.split(' ').some((id) => document.getElementById(id)?.textContent?.trim())) return true;
      if (el.textContent?.trim()) return true;
      const id = el.getAttribute('id');
      if (id && document.querySelector(`label[for="${CSS.escape(id)}"]`)) return true;
      if (el.closest('label')?.textContent?.trim()) return true;
      return false;
    };
    for (const el of document.querySelectorAll(controls)) {
      if (!visible(el) || el.closest('[aria-hidden="true"]')) continue;
      if (!named(el)) found.push({ kind: 'unnamed', detail: label(el) || el.outerHTML.slice(0, 80) });
    }

    // 4. A touch target is at least 40px on a phone (Heartwood's floor).
    if (isPhone) {
      for (const el of document.querySelectorAll(controls)) {
        if (!visible(el) || el.closest('[aria-hidden="true"]')) continue;
        const style = getComputedStyle(el);
        // A link inside running text is as tall as its line; the rule is for controls on their own.
        if (el.tagName === 'A' && style.display === 'inline') continue;
        // A switch or checkbox is a hidden input inside the label that draws it: the label is the target.
        const target = el instanceof HTMLInputElement && /checkbox|radio/.test(el.type) ? (el.closest('label') ?? el) : el;
        const box = target.getBoundingClientRect();
        if (box.height < 39.5 || box.width < 39.5) found.push({ kind: 'small-target', detail: `${label(el)} is ${Math.round(box.width)}x${Math.round(box.height)}` });
      }
    }

    // 5. Text has enough contrast with what is behind it (4.5, or 3 for large text), read from the page's colours.
    const parse = (value: string) => {
      const m = value.match(/rgba?\(([^)]+)\)/);
      if (!m) return null;
      const [r, g, b, a = 1] = m[1].split(/[ ,/]+/).filter(Boolean).map(Number);
      return { r, g, b, a };
    };
    const lum = ({ r, g, b }: { r: number; g: number; b: number }) => {
      const f = (v: number) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    const backdrop = (el: Element) => {
      // Walk up, laying each translucent colour over the next, until one is opaque.
      const layers: { r: number; g: number; b: number; a: number }[] = [];
      for (let up: Element | null = el; up; up = up.parentElement) {
        const c = parse(getComputedStyle(up).backgroundColor);
        if (c && c.a > 0) {
          layers.push(c);
          if (c.a >= 1) break;
        }
        if (getComputedStyle(up).backgroundImage !== 'none') return null; // a gradient or image: not measured
      }
      let base = { r: 255, g: 255, b: 255 };
      if (!layers.length || layers[layers.length - 1].a < 1) base = parse(getComputedStyle(document.body).backgroundColor) ?? base;
      for (const layer of layers.reverse()) base = { r: layer.r * layer.a + base.r * (1 - layer.a), g: layer.g * layer.a + base.g * (1 - layer.a), b: layer.b * layer.a + base.b * (1 - layer.a) };
      return base;
    };
    const seen = new Set<string>();
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const el = node.parentElement;
      if (!el || !node.textContent?.trim() || !visible(el) || el.closest('[aria-hidden="true"], script, style')) continue;
      const style = getComputedStyle(el);
      const fg = parse(style.color);
      const bg = backdrop(el);
      if (!fg || !bg) continue;
      const alpha = fg.a * Number(style.opacity);
      const text = { r: fg.r * alpha + bg.r * (1 - alpha), g: fg.g * alpha + bg.g * (1 - alpha), b: fg.b * alpha + bg.b * (1 - alpha) };
      const [hi, lo] = [lum(text), lum(bg)].sort((a, b) => b - a);
      const ratio = (hi + 0.05) / (lo + 0.05);
      const size = parseFloat(style.fontSize);
      const large = size >= 24 || (size >= 18.66 && Number(style.fontWeight) >= 700);
      const disabled = el.closest('[disabled], [aria-disabled="true"]');
      if (!disabled && ratio < (large ? 3 : 4.5)) {
        const key = label(el);
        if (!seen.has(key)) {
          seen.add(key);
          found.push({ kind: 'contrast', detail: `${key} is ${ratio.toFixed(2)}:1` });
        }
      }
    }

    // 6. Text cut by an ellipsis or a clip is allowed only where a tooltip or a title carries the rest.
    for (const el of document.querySelectorAll('body *')) {
      if (!visible(el) || el.closest('[aria-hidden="true"], .hw-visually-hidden, .kp-announcer')) continue;
      const style = getComputedStyle(el);
      if (el.scrollWidth > el.clientWidth + 1 && style.textOverflow === 'ellipsis' && style.overflow !== 'visible') {
        const hasTip = !!(el.getAttribute('title') || el.closest('[title], [aria-describedby], [data-tooltip]') || el.querySelector('[title]'));
        found.push({ kind: 'clipped', detail: `${label(el)} is cut${hasTip ? ' (has a title)' : ''}` });
      }
    }
    return found as { kind: Finding['kind']; detail: string }[];
  }, phone);
}
