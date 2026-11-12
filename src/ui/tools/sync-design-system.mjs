// Copies the design system in resources/design-system into the React UI library, so the
// design system stays the one source for every token and component style.
//
//   tokens.json            -> src/ui/react-ui/src/styles/tokens.css
//   components/bundle.css  -> src/ui/react-ui/src/styles/components.css
//
// Run from src/ui/: pnpm design:sync. Commit the generated files.

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

// This file sits in src/ui/tools; the repo root is three levels up.
const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const source = join(root, 'resources', 'design-system');
const target = join(root, 'src', 'ui', 'react-ui', 'src', 'styles');

const header = (from) =>
  `/* Generated from resources/design-system/${from} by src/ui/tools/sync-design-system.mjs. Do not edit. */\n`;

const tokens = JSON.parse(readFileSync(join(source, 'tokens.json'), 'utf8'));
const [primary, ...others] = tokens.color.themes.map((t) => t.id);
if (others.length !== 1) {
  throw new Error(`Expected two colour themes, found ${tokens.color.themes.length}.`);
}
const secondary = others[0];

const valueFor = (token, theme) => {
  const value = typeof token.value === 'string' ? token.value : (token.value[theme] ?? token.value[primary]);
  return value.replace(/^\{(.+)\}$/, 'var(--$1)');
};

const themed = [...tokens.color.tokens, ...(tokens.shadow?.tokens ?? [])];
const block = (theme) => themed.map((t) => `  --${t.name}: ${valueFor(t, theme)};`).join('\n');

const fixed = [
  ...Object.entries(tokens.type.families).map(([key, stack]) => `  --font-${key}: ${stack};`),
  ...(tokens.spacing?.tokens ?? []).map((t) => `  --${t.name}: ${t.value};`),
  ...(tokens.radius?.tokens ?? []).map((t) => `  --${t.name}: ${t.value};`),
].join('\n');

const typeClasses = tokens.type.groups
  .flatMap((group) =>
    group.styles.map((style) => {
      const lines = [
        `  font-family: var(--font-${style.family ?? group.family});`,
        `  font-size: ${style.fontSize};`,
        style.lineHeight && `  line-height: ${style.lineHeight};`,
        style.fontWeight && `  font-weight: ${style.fontWeight};`,
        style.letterSpacing && `  letter-spacing: ${style.letterSpacing};`,
      ].filter(Boolean);
      return `.kp-type-${style.name} {\n${lines.join('\n')}\n}`;
    }),
  )
  .join('\n\n');

// The primary theme is the default. The other applies when the OS asks for it and the user has
// not picked a theme, or when the user picks it (data-theme on <html>).
const css = `${header('tokens.json')}
:root {
${block(primary)}
  color-scheme: ${primary};
}

@media (prefers-color-scheme: ${secondary}) {
  :root:not([data-theme="${primary}"]) {
${block(secondary).replace(/^/gm, '  ')}
    color-scheme: ${secondary};
  }
}

:root[data-theme="${secondary}"] {
${block(secondary)}
  color-scheme: ${secondary};
}

:root {
${fixed}
}

${typeClasses}
`;

writeFileSync(join(target, 'tokens.css'), css);

// Fonts are self-hosted by the library (fonts.css), so drop the design system's Google Fonts import.
const components = readFileSync(join(source, 'components', 'bundle.css'), 'utf8')
  .replace(/^@import url\("https:\/\/fonts\.googleapis\.com[^\n]*\n+/m, '');
// The import is cut by its shape. If the design system spells it another way, stop rather than ship a
// stylesheet that asks Google for fonts on a network that may have no internet.
if (components.includes('fonts.googleapis.com')) {
  throw new Error('components/bundle.css still imports Google Fonts: update the pattern in sync-design-system.mjs.');
}
writeFileSync(join(target, 'components.css'), header('components/bundle.css') + components);

console.log(`Wrote ${target}: tokens.css and components.css`);
