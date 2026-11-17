import { mkdirSync, writeFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { measure, watch, type Finding } from './audit';
import { scenarios } from './scenarios';
import { answerSignedOut, serve } from './support';

// The pre-release pass: every screen and state in scenarios.ts, at desktop and phone width (the projects),
// in light and dark. Each is measured (audit.ts) and photographed into test-results/screens, and the findings
// go to a JSON file beside the picture. What breaks a rule fails the test; SCREENS.md lists what was driven.
const out = 'test-results/screens';
const hard = new Set<Finding['kind']>(['sideways', 'console', 'unnamed', 'offscreen', 'small-target', 'contrast']);

// What the pass found that belongs to Heartwood, reported to it and left here until its release fixes it. A
// finding that matches one is written to the report as known and does not fail the run; when Heartwood ships
// the fix the entry is deleted.
const known: { kind: Finding['kind']; match: RegExp; why: string }[] = [
  { kind: 'contrast', match: /^span\.hw-(prop-label-text|visually-hidden) .* is 4\.2\d:1$/, why: 'Heartwood leaves it: the label and hint of a disabled property are 4.2:1, and WCAG exempts inactive controls (the check here is stricter)' },
];

test.use({ actionTimeout: 5000 });

test.beforeEach(async ({ page, isMobile }) => {
  if (isMobile) await page.setViewportSize({ width: 390, height: 844 });
});

for (const scenario of scenarios) {
  for (const scheme of ['light', 'dark'] as const) {
    test(`${scenario.id} (${scheme})`, async ({ page, isMobile }, info) => {
      const phone = !!isMobile;
      const skip = phone ? scenario.skip?.phone : scenario.skip?.desktop;
      test.skip(!!skip, skip);

      await page.emulateMedia({ colorScheme: scheme });
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
      for (const f of findings) {
        const hit = known.find((k) => k.kind === f.kind && k.match.test(f.detail));
        if (hit) f.known = hit.why;
      }
      const name = `${scenario.id}.${phone ? 'phone' : 'desktop'}.${scheme}`;
      mkdirSync(out, { recursive: true });
      await page.screenshot({ path: `${out}/${name}.png`, fullPage: !phone });
      writeFileSync(`${out}/${name}.json`, JSON.stringify({ id: scenario.id, what: scenario.what, width: phone ? 'phone' : 'desktop', scheme, findings }, null, 1));

      const broken = findings.filter((f) => hard.has(f.kind) && !f.known);
      expect(broken, broken.map((f) => `${f.kind}: ${f.detail}`).join('\n')).toEqual([]);
    });
  }
}
