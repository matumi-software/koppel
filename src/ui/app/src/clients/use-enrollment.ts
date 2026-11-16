import { useQuery } from '@tanstack/react-query';
import { useCallback } from 'react';
import { enrollmentKey } from '@/pending/domains';
import { usePending } from '@/pending/pending-context';
import { enrollmentOptions } from '@/queries';

/** The pairing code this server shows, if any, and the revision of its server key (server role). EnrollmentChanged keeps it current. */
export function useEnrollment(enabled = true) {
  return useQuery({ ...enrollmentOptions(), enabled });
}

/**
 * Showing and closing the pairing code under the confirm rule. `open` replaces a code that shows.
 * Read the key itself with `getEnrollmentKey`, and rotate it with `rotateEnrollmentKey` (api/server.ts);
 * both are one-off calls, like a client's secret.
 */
export function useEnrollmentCode() {
  const query = useEnrollment();
  const { pending, failures, request } = usePending('enrollment');
  const open = useCallback(() => request({ type: 'open', previous: query.data?.code?.code ?? null }), [request, query.data]);
  const close = useCallback(() => request({ type: 'close' }), [request]);
  return { code: query.data?.code ?? null, keyRevision: query.data?.keyRevision, isPending: enrollmentKey in pending, failure: failures[enrollmentKey], open, close };
}
