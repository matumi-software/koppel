import type { SystemCheck } from '@/api/generated/wire';

/**
 * The checks that matter to a machine with this role. The service answers with the sharing and the
 * attaching check whatever the role, so a client-only machine would otherwise be told that sharing is
 * not available. The file checks (settings, devices, attached) are kept for every role.
 */
export function checksForRole(checks: readonly SystemCheck[], server: boolean, client: boolean): SystemCheck[] {
  return checks.filter((c) => (c.id === 'share' ? server : c.id === 'attach' ? client : true));
}
