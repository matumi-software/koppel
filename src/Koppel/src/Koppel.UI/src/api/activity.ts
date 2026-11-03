import { getJson } from './http';

/** Mirrors Koppel.Core.Activity.ActivityEvent. */
export interface ServerActivity {
  id: string;
  timestampUtc: string;
  entity: { id: string; name: string; type: 'device' | 'client' | 'machine' | 'link' };
  reason:
    | 'shared'
    | 'unshared'
    | 'attached'
    | 'detached'
    | 'plugged'
    | 'unplugged'
    | 'clientAdded'
    | 'clientRevoked'
    | 'secretRotated'
    | 'reconfigured'
    | 'linkUp'
    | 'linkDown'
    | 'signInFailed'
    | 'passwordChanged'
    | 'signingKeyRotated'
    | 'certificatePrepared'
    | 'certificateCancelled'
    | 'certificateSwitched'
    | 'serverMoved';
  outcome: 'succeeded' | 'failed';
  summary: string;
  detail: string | null;
}

export const getActivity = () => getJson<ServerActivity[]>('/api/activity?max=200');
