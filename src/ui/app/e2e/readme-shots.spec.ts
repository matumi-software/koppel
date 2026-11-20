import { mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { test } from '@playwright/test';
import { scenarios } from './scenarios';
import { answerSignedOut, serve } from './support';

// The pictures in README.md (docs/images), made from the same states the whole-app pass drives, with the service's
// answers stood in. Not part of the run: README_SHOTS=1 pnpm --filter koppel.ui exec playwright test e2e/readme-shots.spec.ts
test.skip(!process.env.README_SHOTS, 'only when the README pictures are being remade');
test.skip(({ isMobile }) => !!isMobile, 'desktop pictures');

const shots = [
  { id: 'devices', file: 'devices' },
  { id: 'clients-code-live', file: 'pairing-code' },
  { id: 'connection-servers', file: 'servers' },
];
const out = fileURLToPath(new URL('../../../../docs/images', import.meta.url));

for (const shot of shots) {
  for (const scheme of ['light', 'dark'] as const) {
    test(`${shot.file} (${scheme})`, async ({ page }) => {
      const scenario = scenarios.find((s) => s.id === shot.id);
      if (!scenario) throw new Error(`No scenario ${shot.id}`);
      await page.setViewportSize({ width: 1120, height: 700 });
      await page.emulateMedia({ colorScheme: scheme });
      if (scenario.role) await serve(page, scenario.role, scenario.options);
      else if (scenario.auth) await answerSignedOut(page, scenario.auth);
      await scenario.prepare?.(page);
      await page.goto(scenario.hash ? `/#${scenario.hash}` : '/');
      await scenario.act?.(page);
      await scenario.ready(page);
      await page.waitForTimeout(900);
      mkdirSync(out, { recursive: true });
      await page.screenshot({ path: `${out}/${shot.file}-${scheme}.png`, animations: 'disabled' });
    });
  }
}
