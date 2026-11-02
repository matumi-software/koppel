import { Frame } from '@/shell/frame';
import { useFrame } from '@/shell/frame-context';
import { ConnectionSection } from './connection-section';

/** Client role: the server this machine attaches devices from. */
export function ConnectionPage() {
  const { locked } = useFrame();
  return (
    <Frame
      title="Server connection"
      lede="The Koppel server this machine attaches devices from. Ask its admin for a client."
    >
      <ConnectionSection locked={locked} />
    </Frame>
  );
}
