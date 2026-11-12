// Generated from the Rust types by `KOPPEL_WRITE_WIRE=1 cargo test -p koppel-web wire`.
// Do not edit: the test `generated_types_are_current` fails when this differs.

export type ActivityEvent = { id: string, timestampUtc: string, entity: EntityRef, reason: ActivityReason, outcome: ActivityOutcome, summary: string, detail: string | null, };

export type ActivityOutcome = "succeeded" | "failed";

export type ActivityReason = "shared" | "unshared" | "attached" | "detached" | "plugged" | "unplugged" | "linkUp" | "linkDown" | "clientAdded" | "clientRevoked" | "secretRotated" | "reconfigured" | "signInFailed" | "passwordChanged" | "certificatePrepared" | "certificateCancelled" | "certificateSwitched" | "serverMoved";

export type AuthState = { signedIn: boolean, passwordSet: boolean, machineName: string, setupOpen: boolean, 
/**
 * Setup asks for the one-time token the log shows.
 */
setupTokenRequired: boolean, setupUntil: string | null, };

export type ClientDeviceView = { device: DeviceView, attachedHere: boolean, 
/**
 * The port this machine's kernel or driver attached it on; `None` for another gateway attach.
 */
port: number | null, attachedSince: string | null, fault: string | null, 
/**
 * Where a gateway attach came from, such as `172.17.0.1`.
 */
attachedVia: string | null, };

export type ClientDevices = { link: LinkState, linkMessage: string | null, devices: Array<ClientDeviceView>, unavailable: string | null, 
/**
 * True while the plain USB/IP gateway listens for a driver Koppel does not drive itself
 * (Koppel in a container on Windows), so the UI can show the command that attaches through it.
 */
gateway: boolean, 
/**
 * Why the gateway is not listening although it should be, such as a port that is taken.
 */
gatewayFault: string | null, };

export type ClientSecret = { clientSecret: string, };

export type ClientView = { id: string, name: string, createdAt: string, lastSeen: string | null, 
/**
 * It has the fingerprint of the certificate prepared for a handover.
 */
hasNextCertificate: boolean, 
/**
 * Its secret is kept, so it can be copied again.
 */
hasSecret: boolean, };

export type ConnectionTest = { ok: boolean, outcome: Outcome, message: string, };

export type ConnectionView = { serverUrl: string | null, serverFingerprint: string | null, clientId: string | null, hasSecret: boolean, nextServerFingerprint: string | null, };

export type DeviceState = "shared" | "attached" | "offline" | "local";

export type DeviceView = { busId: string, vidPid: string, name: string, manufacturer: string | null, speed: string, state: DeviceState, attachedTo: string | null, attachedSince: string | null, 
/**
 * Why the last change to this device failed, for the UI.
 */
fault: string | null, serial: string | null, driver: string | null, interfaces: Array<string> | null, };

export type DiscoveredServer = { name: string, address: string, httpsPort: number, fingerprint: string, version: string | null, otherAddresses: Array<string>, 
/**
 * The address as a URL, as 0.1.x also wrote it.
 */
url: string, };

export type EntityKind = "device" | "client" | "machine" | "link";

export type EntityRef = { id: string, name: string, type: EntityKind, };

export type KeysView = { fingerprint: string, nextFingerprint: string | null, nextPreparedAt: string | null, };

export type KoppelSettings = { role: Role, 
/**
 * Share the devices that were shared before a restart. 0.1.x read a missing value as false.
 */
shareOnStartup: boolean, 
/**
 * False until the first-run setup in the UI is finished.
 */
setupComplete: boolean, 
/**
 * Windows app: the tray says when a client waits to pair.
 */
notifyPairing: boolean, 
/**
 * Windows app: the tray says when the link to the server is lost, and when it is back.
 */
notifyLink: boolean, 
/**
 * Windows app: the tray says when a device could not be attached.
 */
notifyAttachFailures: boolean, };

export type LinkState = "unconfigured" | "connecting" | "connected" | "error";

export type NewClient = { clientId: string, clientSecret: string, name: string, };

export type Outcome = "ok" | "incomplete" | "unreachable" | "certificateMismatch" | "rejectedCredentials" | "clientUnknown" | "versionMismatch" | "unexpectedAnswer" | "tooManyRequests";

export type PairingOverview = { requests: Array<PairingRequestView>, 
/**
 * The server had its fill of requests for now and turns new ones away.
 */
budgetExhausted: boolean, window: PairingWindowView, };

export type PairingRequestView = { id: string, clientName: string, from: string, requestedAt: string, expiresAt: string, state: RequestState, message: string | null, };

export type PairingState = "idle" | "requesting" | "waiting" | "confirm" | "paired" | "denied" | "expired" | "failed";

export type PairingStatus = { state: PairingState, serverName: string | null, serverUrl: string | null, code: string | null, message: string | null, expiresAt: string | null, serverFingerprint: string | null, };

export type PairingWindowView = { open: boolean, 
/**
 * The server takes requests at any time (not something Koppel sets; kept so the UI need not
 * guess if a build ever does).
 */
alwaysOpen: boolean, 
/**
 * When the window closes by itself. `None` when it is closed.
 */
until: string | null, };

export type Problem = { 
/**
 * `about:blank`: the status says it all.
 */
type: string, title: string, status: number, 
/**
 * A stable word to branch on, such as `validation` or `wrong_password`.
 */
code: string, 
/**
 * A sentence the UI shows as it is.
 */
detail: string, 
/**
 * A validation problem: each field with its messages.
 */
errors?: Record<string, Array<string>>, };

export type RequestState = "waiting" | "denied";

export type Role = "server" | "client" | "both";

export type ServerDevices = { devices: Array<DeviceView>, unavailable: string | null, };

export type ServerIdentity = { name: string, fingerprint: string, httpsPort: number, };

export type SystemCheck = { id: string, ok: boolean, title: string, detail: string, };

export type SystemInfo = { name: string, version: string, commit: string, built: string, startedAt: string, role: Role, uiPort: number, httpsPort: number, fingerprint: string, os: string, tray: boolean, };

export type Approve = { code?: string, };

export type ChangeRequest = { current?: string, next?: string, };

export type ConnectionUpdate = { serverUrl?: string, serverFingerprint?: string, clientId?: string, clientSecret?: string, };

export type CreateClient = { name?: string, };

export type OpenPairing = { minutes?: number, };

export type PairStart = { serverUrl?: string, serverFingerprint?: string, serverName?: string, };

export type PasswordRequest = { password?: string, 
/**
 * Setup only: the token the log shows, where one is required.
 */
setupToken?: string, };

export type SettingsPatch = { role?: Role, shareOnStartup?: boolean, setupComplete?: boolean, notifyPairing?: boolean, notifyLink?: boolean, notifyAttachFailures?: boolean, };
