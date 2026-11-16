import type { ThirdPartyNotice } from '@matumi/ui';

/**
 * What the UI ships, with the version in the build and its licence. Taken from the installed
 * packages' own package.json; update it when a dependency of the UI changes.
 */
export const notices: ThirdPartyNotice[] = [
  { name: 'React', version: '19.3.1', licence: 'MIT' },
  { name: 'React Aria Components', version: '1.22.1', licence: 'Apache-2.0' },
  { name: 'TanStack Query', version: '5.105.1', licence: 'MIT' },
  { name: 'Lucide', version: '1.65.0', licence: 'ISC' },
  { name: 'Heartwood', version: '0.1.0', licence: 'PolyForm-Shield-1.0.0' },
  { name: 'Instrument Sans', version: '5.3.0', licence: 'OFL-1.1' },
  { name: 'JetBrains Mono', version: '5.3.0', licence: 'OFL-1.1' },
];
