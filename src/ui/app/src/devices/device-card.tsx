import type { ReactNode } from 'react';
import { DrillLink, Tile, TileGrid } from '@heartwood/ui';

/**
 * One device as a tile of a grid: its name opens it, one line says what it is doing, and the state
 * word sits beside its switch. The same slots in every state, so switching changes the words, not the layout.
 */
export function DeviceCard({ id, name, sub, subDanger, state, control, onOpen }: { id: string; name: string; sub: string; subDanger?: boolean; state: ReactNode; control: ReactNode; onOpen: () => void }) {
  return (
    <Tile title={<DrillLink drillId={id} onPress={onOpen}>{name}</DrillLink>} padded>
      <p className={subDanger ? 'kp-small kp-danger kp-card-sub' : 'kp-muted kp-small kp-card-sub'}>{sub}</p>
      <div className="kp-card-foot">
        {state}
        {control}
      </div>
    </Tile>
  );
}

/** Cards grouped by what the person cannot change from here, so a card never moves when it is switched. */
export function CardGroup({ title, count, children }: { title: string; count: number; children: ReactNode }) {
  return (
    <section className="kp-section">
      <h2 className="kp-h2">
        {title} <span className="kp-muted kp-count">{count}</span>
      </h2>
      <TileGrid label={title}>{children}</TileGrid>
    </section>
  );
}
