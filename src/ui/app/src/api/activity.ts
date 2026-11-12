import { getJson } from './http';

export type { ActivityEvent, ActivityOutcome, ActivityReason, EntityKind, EntityRef } from './generated/wire';
import type { ActivityEvent } from './generated/wire';

export const getActivity = () => getJson<ActivityEvent[]>('/api/activity?max=200');
