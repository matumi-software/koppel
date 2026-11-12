/**
 * The JSON fixtures in `test/wire/`, typed by the types generated from the Rust DTOs. The Rust test
 * `the_fixtures_are_what_rust_sends` (koppel-web, `wire.rs`) reads the same files into the Rust types and
 * writes them back, so the mocks and the server cannot drift apart without a test failing.
 */
import type {
  ActivityEvent,
  AuthState,
  ClientDevices,
  ClientView,
  ConnectionTest,
  ConnectionView,
  DiscoveredServer,
  KeysView,
  KoppelSettings,
  NewClient,
  PairingOverview,
  PairingStatus,
  ServerDevices,
  ServerIdentity,
  SystemCheck,
  SystemInfo,
} from '@/api/generated/wire';
import activityEventJson from './wire/activity-event.json';
import authStateJson from './wire/auth-state.json';
import clientDevicesJson from './wire/client-devices.json';
import clientsJson from './wire/clients.json';
import connectionTestJson from './wire/connection-test.json';
import connectionViewJson from './wire/connection-view.json';
import discoveredServersJson from './wire/discovered-servers.json';
import keysViewJson from './wire/keys-view.json';
import newClientJson from './wire/new-client.json';
import pairingOverviewJson from './wire/pairing-overview.json';
import pairingStatusJson from './wire/pairing-status.json';
import serverDevicesJson from './wire/server-devices.json';
import serverIdentityJson from './wire/server-identity.json';
import settingsJson from './wire/settings.json';
import systemChecksJson from './wire/system-checks.json';
import systemInfoJson from './wire/system-info.json';

/** What TypeScript makes of a JSON file: the words of a union are plain strings. */
type Widen<T> = T extends string ? string : T extends readonly (infer U)[] ? Widen<U>[] : T extends object ? { [K in keyof T]: Widen<T[K]> } : T;

/**
 * Checks a fixture has every field of the generated type with the right kind of value, then gives it
 * that type. A field missing or of another type fails the typecheck. The words of an enum cannot be
 * checked here (JSON imports widen them); the Rust test reads them into the Rust enums.
 */
const wire = <T,>(fixture: Widen<T>): T => fixture as T;

export const activityEvent = wire<ActivityEvent>(activityEventJson);
export const authState = wire<AuthState>(authStateJson);
export const clientDevices = wire<ClientDevices>(clientDevicesJson);
export const clients = wire<ClientView[]>(clientsJson);
export const connectionTest = wire<ConnectionTest>(connectionTestJson);
export const connectionView = wire<ConnectionView>(connectionViewJson);
export const discoveredServers = wire<DiscoveredServer[]>(discoveredServersJson);
export const keysView = wire<KeysView>(keysViewJson);
export const newClient = wire<NewClient>(newClientJson);
export const pairingOverview = wire<PairingOverview>(pairingOverviewJson);
export const pairingStatus = wire<PairingStatus>(pairingStatusJson);
export const serverDevices = wire<ServerDevices>(serverDevicesJson);
export const serverIdentity = wire<ServerIdentity>(serverIdentityJson);
export const settings = wire<KoppelSettings>(settingsJson);
export const systemChecks = wire<SystemCheck[]>(systemChecksJson);
export const systemInfo = wire<SystemInfo>(systemInfoJson);
