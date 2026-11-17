import { mkdirSync, writeFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { measure, watch, type Finding } from './audit';
import { scenarios } from './scenarios';
import { serve } from './support';

// The widths around the 800 px breakpoint, where the settings panel turns into a sheet and the tables drop
// columns: every page and state a signed-in role has, at each width, in light. The two project widths are
// covered by screens.spec.ts. Touch targets are not measured here: that rule is for a phone.
const widths = [640, 768, 799, 800, 1024];
const out = 'test-results/widths';
const hard = new Set<Finding['kind']>(['sideways', 'console', 'unnamed', 'offscreen', 'contrast']);
const known: { kind: Finding['kind']; match: RegExp }[] = [{ kind: 'contrast', match: /^span\.hw-(prop-label-text|visually-hidden) .* is 4\.2\d:1$/ }];

test.use({ actionTimeout: 5000 });
test.skip(({ isMobile }) => isMobile, 'sets its own widths');

for (const width of widths) {
  for (const scenario of scenarios.filter((s) => s.role && s.hash && s.options?.link !== 'down')) {
    test(`${scenario.id} at ${width}`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: 'light' });
      const logged = watch(page);
      await serve(page, scenario.role!, scenario.options);
      await scenario.prepare?.(page);
      await page.goto(`/#${scenario.hash}`);
      await scenario.act?.(page).catch(async (error) => {
        await page.screenshot({ path: info.outputPath('stuck.png') });
        throw error;
      });
      await scenario.ready(page);
      await page.waitForTimeout(900);

      const findings = [...(scenario.logsErrors ? [] : logged), ...(await measure(page, false))].filter((f) => f.kind !== 'small-target');
      mkdirSync(out, { recursive: true });
      const name = `${scenario.id}.${width}`;
      await page.screenshot({ path: `${out}/${name}.png` });
      writeFileSync(`${out}/${name}.json`, JSON.stringify({ id: scenario.id, what: scenario.what, width, findings }, null, 1));
      const broken = findings.filter((f) => hard.has(f.kind) && !known.some((k) => k.kind === f.kind && k.match.test(f.detail)));
      expect(broken, broken.map((f) => `${f.kind}: ${f.detail}`).join('\n')).toEqual([]);
    });
  }
}
