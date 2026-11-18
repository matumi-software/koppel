import { mkdirSync, writeFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { hoverCut, measure, watch, type Finding } from './audit';
import { scenarios } from './scenarios';
import { answerSignedOut, serve } from './support';

// The pre-release pass: every screen and state in scenarios.ts, at desktop and phone width (the projects), in
// light and dark and, with colour vision mode on, light-safe and dark-safe. Each is measured (audit.ts) and
// photographed into test-results/screens, and the findings go to a JSON file beside the picture. On desktop each
// cut value is also hovered, to see that a tooltip carries its full text. What breaks a rule fails the test;
// SCREENS.md lists what was driven.
const out = 'test-results/screens';
const hard = new Set<Finding['kind']>(['sideways', 'console', 'unnamed', 'offscreen', 'small-target', 'contrast', 'no-tooltip']);

// What the pass found that belongs to Heartwood, reported to it and left here until its release fixes it. A
// finding that matches one is written to the report as known and does not fail the run; when Heartwood ships
// the fix the entry is deleted.
const known: { kind: Finding['kind']; match: RegExp; why: string }[] = [
  { kind: 'contrast', match: /^span\.hw-(prop-label-text|visually-hidden) .* is 4\.2\d:1$/, why: 'Heartwood leaves it: the label and hint of a disabled property are 4.2:1, and WCAG exempts inactive controls (the check here is stricter)' },
  { kind: 'small-target', match: /^a\.hw-skip "Skip to content" is \d+x3[0-9]$/, why: 'Heartwood: the skip link is 38 px high on a phone (the minimum is 40); it shows only while focused. Reported to Heartwood' },
];

test.use({ actionTimeout: 5000 });

test.beforeEach(async ({ page, isMobile }) => {
  if (isMobile) await page.setViewportSize({ width: 390, height: 844 });
});

for (const scenario of scenarios) {
  for (const scheme of ['light', 'dark', 'light-safe', 'dark-safe'] as const) {
    test(`${scenario.id} (${scheme})`, async ({ page, isMobile }, info) => {
      const phone = !!isMobile;
      const skip = phone ? scenario.skip?.phone : scenario.skip?.desktop;
      test.skip(!!skip, skip);

      const base = scheme.startsWith('dark') ? 'dark' : 'light';
      await page.emulateMedia({ colorScheme: base });
      // Colour vision mode is the look this browser stored, as the Settings page would have left it.
      if (scheme.endsWith('-safe')) await page.addInitScript((theme) => localStorage.setItem('koppel.look', JSON.stringify({ theme, colourVision: true })), base);
      const logged = watch(page);
      if (scenario.options?.link === 'down') await page.clock.install();
      if (scenario.role) await serve(page, scenario.role, scenario.options);
      else if (scenario.auth) await answerSignedOut(page, scenario.auth);
      await scenario.prepare?.(page);

      await page.goto(scenario.hash ? `/#${scenario.hash}` : '/');
      await scenario.act?.(page).catch(async (error) => {
        // The state could not be reached: say so with a picture of where it stopped.
        await page.screenshot({ path: info.outputPath('stuck.png') });
        throw error;
      });
      await scenario.ready(page).catch(async (error) => {
        await page.screenshot({ path: info.outputPath('stuck.png') });
        throw error;
      });
      // Let transitions and the focus move settle before measuring.
      await page.waitForTimeout(900);

      const findings: (Finding & { known?: string })[] = [...(scenario.logsErrors ? [] : logged), ...(await measure(page, phone))];
      const name = `${scenario.id}.${phone ? 'phone' : 'desktop'}.${scheme}`;
      mkdirSync(out, { recursive: true });
      await page.screenshot({ path: `${out}/${name}.png`, fullPage: !phone });
      if (!phone) findings.push(...(await hoverCut(page)));
      for (const f of findings) {
        const hit = known.find((k) => k.kind === f.kind && k.match.test(f.detail));
        if (hit) f.known = hit.why;
      }
      // The look the page really has: the stored choice reaches the root element.
      expect(await page.evaluate(() => document.documentElement.dataset.theme), 'the theme on the page').toBe(scheme);
      writeFileSync(`${out}/${name}.json`, JSON.stringify({ id: scenario.id, what: scenario.what, width: phone ? 'phone' : 'desktop', scheme, findings }, null, 1));

      const broken = findings.filter((f) => hard.has(f.kind) && !f.known);
      expect(broken, broken.map((f) => `${f.kind}: ${f.detail}`).join('\n')).toEqual([]);
    });
  }
}
