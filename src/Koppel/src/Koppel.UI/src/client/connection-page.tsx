import { ActivityFeed } from '@koppel/react-ui';
import { toEntries, useActivity } from '@/activity/activity-context';
import { Frame } from '@/shell/frame';
import { useFrame } from '@/shell/frame-context';
import { ConnectionSection } from './connection-section';

/** Client role: the server this machine attaches devices from. */
export function ConnectionPage() {
  const { locked } = useFrame();
  const { items } = useActivity();
  return (
    <Frame
      title="Server connection"
      lede="The Koppel server this machine attaches devices from. Ask its admin for a client."
      aside={
        <>
          <h2 className="kp-type-heading" style={{ margin: 0 }}>
            Activity
          </h2>
          <ActivityFeed entries={toEntries(items)} empty="Changes show here when the server confirms or rejects them." />
        </>
      }
    >
      <ConnectionSection locked={locked} />
    </Frame>
  );
}
