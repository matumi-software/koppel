export type DeviceState = 'shared' | 'attached' | 'pending' | 'offline' | 'local';

const words: Record<DeviceState, string> = {
  shared: 'Shared',
  attached: 'Attached',
  pending: 'Pending',
  offline: 'Offline',
  local: 'Not shared',
};

export interface StatusBadgeProps {
  state: DeviceState;
  /** Optional detail after the word, such as the client name. */
  detail?: string;
}

/** Names a device's state with its fixed word and glyph. */
export function StatusBadge({ state, detail }: StatusBadgeProps) {
  return (
    <span className="kp-badge" data-state={state}>
      {detail ? `${words[state]} · ${detail}` : words[state]}
    </span>
  );
}
