import { useMemo } from 'react';

/** The words to show for a failed request: the server's own sentence when it gave one. */
export const reasonOf = (error: unknown, fallback: string) => (error instanceof Error ? error.message : fallback);

/**
 * A failed request as the `validationErrors` of a react-aria Form: the server's refusal under the
 * field it is about. The same object until the error changes, so react-aria can clear it when the
 * person edits the field without it coming back on the next render.
 */
export function useServerErrors(error: unknown, field: string | undefined, fallback: string): Record<string, string> {
  return useMemo(() => (error && field ? { [field]: reasonOf(error, fallback) } : {}), [error, field, fallback]);
}
