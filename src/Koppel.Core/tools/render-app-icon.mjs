// Renders the app icon in resources/design-system/assets/Logos for Windows:
//
//   koppel-app-icon.svg -> koppel-app-icon.ico      (16 to 256 px: Apps list, setup.exe, Koppel.exe)
//                       -> koppel-app-icon-64.png   (the logo in the setup window)
//
// Run from src/: pnpm design:icons. Commit the generated files.

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';

// This file sits in src/Koppel.Core/tools; the repo root is three levels up.
const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const logos = join(root, 'resources', 'design-system', 'assets', 'Logos');
const svg = readFileSync(join(logos, 'koppel-app-icon.svg'));

const render = (size) => new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render().asPng();

// An .ico whose images are PNGs, which Windows reads since Vista: a 6-byte header, a 16-byte entry per image, then the images.
const sizes = [16, 20, 24, 32, 40, 48, 64, 256];
const images = sizes.map(render);
const header = Buffer.alloc(6 + 16 * sizes.length);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(sizes.length, 4);
let offset = header.length;
sizes.forEach((size, i) => {
  const entry = 6 + 16 * i;
  header.writeUInt8(size === 256 ? 0 : size, entry);
  header.writeUInt8(size === 256 ? 0 : size, entry + 1);
  header.writeUInt16LE(1, entry + 4);
  header.writeUInt16LE(32, entry + 6);
  header.writeUInt32LE(images[i].length, entry + 8);
  header.writeUInt32LE(offset, entry + 12);
  offset += images[i].length;
});

writeFileSync(join(logos, 'koppel-app-icon.ico'), Buffer.concat([header, ...images]));
writeFileSync(join(logos, 'koppel-app-icon-64.png'), render(64));
console.log(`Rendered koppel-app-icon.ico (${sizes.join(', ')} px) and koppel-app-icon-64.png`);
