import { expect, type Page } from '@playwright/test';
import { signedOut, wire, type ServeOptions } from './support';

// Every screen and state of the app, as data: how to reach it with the service's answers stood in, and what
// shows when it is there. screens.spec.ts drives each one at both widths in light and dark and measures it.
// A state that needs a change from the service sends it with `window.__push(name, data)`, as the real push does.

type Role = 'server' | 'client' | 'both';

export interface Scenario {
  id: string;
  /** What the screen is, for SCREENS.md. */
  what: string;
  /** Where in the app it is: signed out, or a role's page. Leave out for a signed-out screen. */
  role?: Role;
  options?: ServeOptions;
  /** The answer to the sign-in state, for a signed-out screen. */
  auth?: object;
  /** What to set up before the page opens: an answer to a request that is not the fixture's. */
  prepare?: (page: Page) => Promise<unknown>;
  hash?: string;
  /** What to do once the page is open, to reach the state. */
  act?: (page: Page) => Promise<void>;
  /** Waits until the state is on screen. */
  ready: (page: Page) => Promise<void>;
  /** The page logs errors on purpose here (a failed request is the state). */
  logsErrors?: boolean;
  /** Skip a width, with the reason. */
  skip?: { desktop?: string; phone?: string };
}

const text = (page: Page, value: string | RegExp) => expect(page.getByText(value).first()).toBeVisible();
const heading = (page: Page, name: string | RegExp) => expect(page.getByRole('heading', { name }).first()).toBeVisible();
const click = (page: Page, name: string | RegExp, role: 'button' | 'link' = 'button') => page.getByRole(role, { name }).first().click();

// Pieces of the fixtures, changed for a state.
const devices = wire('server-devices');
const base = devices.devices[0];
const server = (...list: object[]) => ({ ...devices, devices: list });
const device = (patch: object) => ({ ...base, ...patch });
const clientDevices = wire('client-devices');
const remoteEntry = clientDevices.servers[0].devices[0];
const awayServer = (patch: object, list: object[] = []) => ({ id: 'server-2', name: 'cellar', link: 'connected', linkMessage: null, devices: list, ...patch });
const clientView = (...servers: object[]) => ({ ...clientDevices, servers });
const checks = wire('system-checks');
const aMinuteFromNow = () => new Date(Date.now() + 5 * 60_000).toISOString();

const answers = (path: string, value: unknown) => ({ [`/api/v1/${path}`]: value });
const withClients = (list: object[]) => answers('server/clients', list);

const refuse = (page: Page, method: string, url: string, status: number, body: object) =>
  page.route(url, (route) => (route.request().method() === method ? route.fulfill({ status, contentType: 'application/problem+json', json: body }) : route.fallback()));
const reply = (page: Page, method: string, url: string, status: number, body?: object) =>
  page.route(url, (route) => (route.request().method() === method ? route.fulfill(body ? { status, json: body } : { status, body: '' }) : route.fallback()));

const long = 'a-very-long-name-that-has-no-sensible-place-to-break-'.repeat(3);

export const scenarios: Scenario[] = [
  // ------------------------------------------------------------------ before sign-in
  {
    id: 'gate-loading', what: 'The sign-in state has not answered yet: a skeleton.',
    prepare: (page) => page.route('**/api/v1/auth/state', () => new Promise(() => {})),
    // The skeleton once collapsed to nothing in the centred window, so the screen was blank.
    ready: async (page) => { await expect(page.locator('.hw-skeleton').first()).toBeVisible(); expect((await page.locator('.hw-skeleton').first().boundingBox())!.width).toBeGreaterThan(50); },
  },
  {
    id: 'gate-could-not-reach', what: 'The service does not answer the sign-in state.', logsErrors: true,
    prepare: (page) => page.route('**/api/v1/auth/state', (route) => route.fulfill({ status: 500, body: '' })),
    ready: async (page) => { await heading(page, 'Could not reach Koppel'); await expect(page.getByRole('button', { name: 'Try again' })).toBeVisible(); },
  },
  {
    id: 'first-password', what: 'First run: the admin password, with the setup token field.', auth: signedOut,
    ready: async (page) => { await heading(page, 'Set up bench-pc'); await text(page, 'Step 1 of 2'); },
  },
  {
    id: 'first-password-token', what: 'First run when the service wants the setup token.', auth: { ...signedOut, setupTokenRequired: true },
    ready: async (page) => { await expect(page.getByLabel('Setup token')).toBeVisible(); },
  },
  {
    id: 'first-password-errors', what: 'First run: the password rules shown after a short password.', auth: signedOut,
    act: async (page) => { await page.getByLabel('New password').fill('short'); await page.getByLabel('Repeat the password').fill('other'); await click(page, 'Set password and continue'); },
    ready: async (page) => { await text(page, 'Use at least 10 characters.'); },
  },
  {
    id: 'first-password-refused', what: 'First run: the service refuses the password.', auth: signedOut, logsErrors: true,
    prepare: (page) => refuse(page, 'POST', '**/api/v1/auth/setup', 409, wire('problem')),
    act: async (page) => { await page.getByLabel('New password').fill('a long enough password'); await page.getByLabel('Repeat the password').fill('a long enough password'); await click(page, 'Set password and continue'); },
    ready: async (page) => { await text(page, /Rotate the secret|secrets were kept/); },
  },
  {
    id: 'first-password-pending', what: 'First run: the password is being sent.', auth: signedOut,
    prepare: (page) => page.route('**/api/v1/auth/setup', (route) => (route.request().method() === 'POST' ? new Promise(() => {}) : route.fallback())),
    act: async (page) => { await page.getByLabel('New password').fill('a long enough password'); await page.getByLabel('Repeat the password').fill('a long enough password'); await click(page, 'Set password and continue'); },
    ready: async (page) => { await page.waitForTimeout(300); },
  },
  {
    id: 'setup-closed', what: 'The first-password window has timed out.', auth: { ...signedOut, setupOpen: false },
    ready: async (page) => { await heading(page, 'Setup has timed out'); await text(page, 'docker compose restart koppel'); },
  },
  {
    id: 'login', what: 'Sign in with the admin password.', auth: { ...signedOut, passwordSet: true, setupOpen: false },
    ready: async (page) => { await heading(page, 'Sign in to bench-pc'); },
  },
  {
    id: 'login-blank', what: 'Sign in with nothing typed.', auth: { ...signedOut, passwordSet: true, setupOpen: false },
    act: async (page) => { await page.locator('input[type=password]').fill(' '); await page.keyboard.press('Enter'); },
    ready: async (page) => { await text(page, 'Enter the admin password.'); },
  },
  {
    id: 'login-refused', what: 'Sign in with a wrong password.', auth: { ...signedOut, passwordSet: true, setupOpen: false }, logsErrors: true,
    prepare: (page) => refuse(page, 'POST', '**/api/v1/auth/login', 401, { title: 'Unauthorized', status: 401, detail: 'That password is not right. Try again in 5 seconds.' }),
    act: async (page) => { await page.locator('input[type=password]').fill('wrong password here'); await page.keyboard.press('Enter'); },
    ready: async (page) => { await text(page, 'That password is not right.'); },
  },
  {
    id: 'session-ended', what: 'A request is answered 401: the sign-in shows at once.', role: 'both', hash: 'check', logsErrors: true,
    act: async (page) => { await text(page, 'All ready'); await page.route('**/api/v1/system/checks', (route) => route.fulfill({ status: 401, body: '' })); await click(page, 'Check again'); },
    ready: async (page) => { await heading(page, 'Sign in to bench-pc'); },
  },
  {
    id: 'app-loading', what: 'Signed in, the settings have not arrived.', role: 'both', options: { link: 'reconnecting' },
    prepare: (page) => page.route('**/api/v1/settings', () => new Promise(() => {})),
    ready: async (page) => { await expect(page.locator('.hw-skeleton').first()).toBeVisible(); expect((await page.locator('.hw-skeleton').first().boundingBox())!.width).toBeGreaterThan(50); },
  },
  {
    id: 'app-could-not-reach', what: 'Signed in, the settings do not arrive.', role: 'both', options: { link: 'reconnecting' }, logsErrors: true,
    prepare: (page) => page.route('**/api/v1/settings', (route) => route.fulfill({ status: 500, body: '' })),
    ready: async (page) => { await heading(page, 'Could not reach Koppel'); },
  },
  {
    id: 'role-step', what: 'First run, step 2: what the computer does, with a failing check.', role: 'server', options: { settings: { setupComplete: false }, answers: answers('system/checks', checks) },
    ready: async (page) => { await heading(page, /What does .* do\?/); await text(page, 'Step 2 of 2'); },
  },
  {
    id: 'role-step-client', what: 'First run, step 2, with Client chosen: the attach check fails.', role: 'server', options: { settings: { setupComplete: false }, answers: answers('system/checks', checks) },
    act: async (page) => { await page.getByRole('radio', { name: /Attach devices from a server/ }).check({ force: true }); },
    ready: async (page) => { await text(page, /vhci-hcd/); await text(page, 'Koppel can save this role'); },
  },
  {
    id: 'role-step-locked', what: 'First run, step 2, while the live link is down.', role: 'server', options: { settings: { setupComplete: false }, link: 'reconnecting' },
    ready: async (page) => { await heading(page, /What does .* do\?/); },
  },
  {
    id: 'role-step-pending', what: 'First run, step 2: Continue pressed, waiting for the service.', role: 'server', options: { settings: { setupComplete: false } },
    act: async (page) => { await click(page, 'Continue'); },
    ready: async (page) => { await page.waitForTimeout(400); },
  },
  {
    id: 'error-boundary', what: 'A page throws while drawing.', role: 'server', hash: 'devices', options: { answers: answers('server/devices', []) }, logsErrors: true,
    ready: async (page) => { await heading(page, 'Something went wrong'); await expect(page.getByRole('button', { name: 'Reload' })).toBeVisible(); },
  },

  // ------------------------------------------------------------------ the shell
  {
    id: 'locked-reconnecting', what: 'The live link has not opened: a notice above the page, changes paused.', role: 'both', hash: 'devices', options: { link: 'reconnecting' },
    ready: async (page) => { await text(page, 'Reconnecting to the Koppel service'); },
  },
  {
    id: 'locked-popover', what: 'The link popover from the strip.', role: 'both', hash: 'overview', options: { link: 'reconnecting' },
    act: async (page) => { await page.locator('header, [class*="strip"]').getByRole('button', { name: /Reconnecting/ }).first().click().catch(() => page.getByRole('button', { name: /Reconnecting/ }).first().click()); },
    ready: async (page) => { await text(page, 'Changes are paused'); },
  },
  {
    id: 'locked-down', what: 'The link is called down after repeated failures.', role: 'both', hash: 'clients', options: { link: 'down' }, logsErrors: true,
    act: async (page) => { await text(page, 'Reconnecting to the Koppel service'); await page.clock.runFor(20_000); },
    ready: async (page) => { await text(page, 'Not connected to the Koppel service'); },
  },

  // ------------------------------------------------------------------ overview
  { id: 'overview-both', what: 'Overview of a server and client, with the fixtures faults.', role: 'both', hash: 'overview', ready: async (page) => { await text(page, 'Needs you'); } },
  { id: 'overview-server', what: 'Overview of a server.', role: 'server', hash: 'overview', ready: async (page) => { await text(page, 'Needs you'); } },
  { id: 'overview-client', what: 'Overview of a client.', role: 'client', hash: 'overview', ready: async (page) => { await text(page, 'Needs you'); } },
  {
    id: 'overview-calm', what: 'Overview with nothing needing attention.', role: 'both', hash: 'overview',
    options: { answers: { ...answers('server/devices', server(device({}))), ...answers('server/pairing', { requests: [], budgetExhausted: false, window: { open: false, alwaysOpen: false, until: null } }), ...answers('client/devices', clientView(clientDevices.servers[0])) } },
    ready: async (page) => { await text(page, 'Nothing needs you'); },
  },
  {
    id: 'overview-empty', what: 'Overview of a server and client with nothing paired or plugged in.', role: 'both', hash: 'overview',
    options: { answers: { ...answers('server/devices', server()), ...withClients([]), ...answers('client/servers', { servers: [] }), ...answers('client/devices', clientView()) } },
    ready: async (page) => { await text(page, 'Plug in a USB device to share it.'); },
  },
  {
    id: 'overview-server-unreachable', what: 'Overview when a server cannot be reached.', role: 'client', hash: 'overview',
    options: { answers: answers('client/devices', clientView({ ...clientDevices.servers[0], link: 'error', linkMessage: 'Connection refused.' })) },
    ready: async (page) => { await text(page, /Cannot reach/); },
  },
  {
    id: 'overview-long', what: 'Overview with very long names.', role: 'both', hash: 'overview',
    options: { answers: { ...answers('server/devices', server(device({ name: long }), device({ busId: '1-2', name: long, state: 'local', attachedTo: null, fault: long }))), ...withClients([{ ...wire('clients')[0], name: long }]) } },
    ready: async (page) => { await text(page, 'Needs you'); },
  },

  // ------------------------------------------------------------------ devices (server)
  {
    id: 'devices', what: 'Devices of a server: attached, shared, not shared with a fault, offline.', role: 'server', hash: 'devices',
    options: { answers: answers('server/devices', server(base, device({ busId: '1-3', name: 'Barcode scanner', manufacturer: 'Zebra', state: 'shared', attachedTo: null, attachedSince: null, fault: null }), { ...device({ busId: '1-2', name: 'Flash drive', manufacturer: null, state: 'local', attachedTo: null, attachedSince: null, fault: 'The device is busy.' }) }, device({ busId: '1-5', name: 'Label printer', state: 'offline', attachedTo: null, attachedSince: null, fault: null }))) },
    ready: async (page) => { await text(page, 'Barcode scanner'); await text(page, 'Label printer'); },
  },
  { id: 'devices-empty', what: 'No device is plugged in.', role: 'server', hash: 'devices', options: { answers: answers('server/devices', server()) }, ready: async (page) => { await text(page, 'Plug in a USB device to share it'); } },
  {
    id: 'devices-no-match', what: 'A search that matches nothing.', role: 'server', hash: 'devices',
    act: async (page) => { await page.getByLabel('Search devices').fill('zzzz'); },
    ready: async (page) => { await text(page, 'No devices match'); },
  },
  {
    id: 'devices-unavailable', what: 'Sharing is not available on this machine.', role: 'server', hash: 'devices',
    options: { answers: answers('server/devices', { ...devices, unavailable: 'usbip-host is not loaded. Load the module, then reload this page.' }) },
    ready: async (page) => { await text(page, 'Devices cannot be shared'); },
  },
  {
    id: 'devices-pending', what: 'A device is being shared: the switch has been turned, no confirmation yet.', role: 'server', hash: 'devices',
    act: async (page) => { await page.getByRole('switch', { name: 'Share Flash drive' }).click({ force: true }); },
    ready: async (page) => { await text(page, 'Sharing'); },
  },
  {
    id: 'devices-stop-dialog', what: 'Stopping the sharing of a device someone is using: the confirmation.', role: 'server', hash: 'devices',
    act: async (page) => { await page.getByRole('switch', { name: 'Share USB Receiver' }).click({ force: true }); },
    ready: async (page) => { await expect(page.getByRole('alertdialog', { name: /Stop sharing/ })).toBeVisible(); },
  },
  {
    id: 'devices-failed', what: 'Sharing failed: the fault comes back with the push.', role: 'server', hash: 'devices',
    act: async (page) => {
      await page.getByRole('switch', { name: 'Share Flash drive' }).click({ force: true });
      await page.evaluate((d) => (window as unknown as { __push: (n: string, x: unknown) => void }).__push('DevicesChanged', d), { ...devices, devices: [base, { ...devices.devices[1], fault: 'The kernel refused to bind the device.' }] });
    },
    ready: async (page) => { await text(page, 'The kernel refused to bind the device.'); },
  },
  {
    id: 'devices-locked', what: 'Devices while the live link is down: switches disabled.', role: 'server', hash: 'devices', options: { link: 'reconnecting' },
    ready: async (page) => { await expect(page.getByRole('switch', { name: 'Share Flash drive' })).toBeDisabled(); },
  },
  { id: 'device', what: 'One device, attached.', role: 'server', hash: 'devices', act: async (page) => { await click(page, 'USB Receiver', 'link').catch(() => page.getByText('USB Receiver').first().click()); }, ready: async (page) => { await text(page, 'Details'); await text(page, 'Bus ID'); } },
  { id: 'device-fault', what: 'One device that is not shared, with its fault.', role: 'server', hash: 'devices', act: async (page) => { await page.getByText('Flash drive').first().click(); }, ready: async (page) => { await text(page, 'Flash drive is not shared'); } },
  {
    id: 'device-gone', what: 'The opened device is unplugged.', role: 'server', hash: 'devices',
    act: async (page) => { await page.getByText('Flash drive').first().click(); await text(page, 'Details'); await page.evaluate((d) => (window as unknown as { __push: (n: string, x: unknown) => void }).__push('DevicesChanged', d), server(base)); },
    ready: async (page) => { await text(page, 'Device not found'); },
  },

  // ------------------------------------------------------------------ remote devices (client)
  {
    id: 'remote', what: 'Remote devices: attached here, available, in use, offline; the gateway callout.', role: 'both', hash: 'remote',
    options: { answers: answers('client/devices', clientView({ ...clientDevices.servers[0], devices: [remoteEntry, { ...remoteEntry, device: { ...remoteEntry.device, busId: '1-6', name: 'Scale', state: 'shared' }, attachedHere: false, port: null }, { ...remoteEntry, device: { ...remoteEntry.device, busId: '1-7', name: 'Camera', state: 'attached', attachedTo: 'bench-2' }, attachedHere: false, port: null }, { ...remoteEntry, device: { ...remoteEntry.device, busId: '1-8', name: 'Dongle', state: 'offline' }, attachedHere: false, port: null }] })) },
    ready: async (page) => { await text(page, 'Scale'); await text(page, 'Camera'); },
  },
  { id: 'remote-empty', what: 'No server is paired.', role: 'client', hash: 'remote', options: { answers: { ...answers('client/servers', { servers: [] }), ...answers('client/devices', clientView()) } }, ready: async (page) => { await text(page, 'Add a server to attach its devices'); } },
  {
    id: 'remote-unreachable', what: 'A server cannot be reached.', role: 'client', hash: 'remote',
    options: { answers: answers('client/devices', clientView({ ...clientDevices.servers[0], link: 'error', linkMessage: 'Connection refused.' })) },
    ready: async (page) => { await text(page, /Cannot reach/); },
  },
  {
    id: 'remote-connecting', what: 'A server is still connecting.', role: 'client', hash: 'remote',
    options: { answers: answers('client/devices', clientView({ ...clientDevices.servers[0], link: 'connecting' })) },
    ready: async (page) => { await text(page, /Connecting to/); },
  },
  {
    id: 'remote-two-servers', what: 'Two servers, one reachable and one not.', role: 'client', hash: 'remote',
    options: { answers: { ...answers('client/servers', { servers: [...wire('servers').servers, { ...wire('servers').servers[0], id: 'server-2', name: 'cellar', serverUrl: 'https://cellar:5241' }] }), ...answers('client/devices', clientView(clientDevices.servers[0], awayServer({ link: 'error', linkMessage: 'No route to host.' }))) } },
    ready: async (page) => { await text(page, /On lab and cellar|On 2 servers/); },
  },
  { id: 'remote-no-match', what: 'A search that matches nothing.', role: 'client', hash: 'remote', act: async (page) => { await page.getByLabel('Search devices').fill('zzzz'); }, ready: async (page) => { await text(page, 'No devices match'); } },
  {
    id: 'remote-unavailable', what: 'Attaching is not available on this machine.', role: 'client', hash: 'remote',
    options: { answers: answers('client/devices', { ...clientDevices, unavailable: 'vhci-hcd is not loaded.' }) },
    ready: async (page) => { await text(page, 'Devices cannot be attached'); },
  },
  {
    id: 'remote-pending', what: 'A device is being detached: the switch has been turned.', role: 'client', hash: 'remote',
    act: async (page) => { await page.getByRole('switch', { name: /Attach USB Receiver/ }).click({ force: true }); },
    ready: async (page) => { await text(page, /Detaching|Attaching/); },
  },
  { id: 'remote-locked', what: 'Remote devices while the link is down.', role: 'client', hash: 'remote', options: { link: 'reconnecting' }, ready: async (page) => { await text(page, 'Reconnecting to the Koppel service'); } },
  { id: 'remote-device', what: 'One remote device, attached here.', role: 'client', hash: 'remote', act: async (page) => { await page.getByText('USB Receiver').first().click(); }, ready: async (page) => { await text(page, 'Details'); await text(page, 'Virtual port'); } },
  {
    id: 'remote-device-gateway', what: 'A shared remote device with the USB/IP driver command.', role: 'client', hash: 'remote',
    options: { answers: answers('client/devices', { ...clientDevices, gateway: true, gatewayFault: null, servers: [{ ...clientDevices.servers[0], devices: [{ ...remoteEntry, attachedHere: false, port: null, device: { ...remoteEntry.device, state: 'shared' } }] }] }) },
    act: async (page) => { await page.getByText('USB Receiver').first().click(); },
    ready: async (page) => { await text(page, 'With the USB/IP driver'); },
  },
  {
    id: 'remote-device-in-use', what: 'A remote device another client is using.', role: 'client', hash: 'remote',
    options: { answers: answers('client/devices', clientView({ ...clientDevices.servers[0], devices: [{ ...remoteEntry, attachedHere: false, port: null, device: { ...remoteEntry.device, state: 'attached', attachedTo: 'bench-2' } }] })) },
    act: async (page) => { await page.getByText('USB Receiver').first().click(); },
    ready: async (page) => { await text(page, /Another client of/); },
  },
  {
    id: 'remote-device-gone', what: 'The opened remote device disappears.', role: 'client', hash: 'remote',
    act: async (page) => { await page.getByText('USB Receiver').first().click(); await text(page, 'Details'); await page.evaluate((d) => (window as unknown as { __push: (n: string, x: unknown) => void }).__push('ClientDevicesChanged', d), clientView(awayServer({ id: 'server-1', name: 'lab' }))); },
    ready: async (page) => { await text(page, 'Device not found'); },
  },

  // ------------------------------------------------------------------ clients (server)
  { id: 'clients', what: 'Clients: the fixture state (pairing window open, a waiting and a refused request, two computers).', role: 'server', hash: 'clients', ready: async (page) => { await text(page, 'bench-laptop'); } },
  { id: 'clients-code-live', what: 'A pairing code with time left.', role: 'server', hash: 'clients', options: { answers: answers('server/enrollment', { code: { code: '48312290', expiresAt: aMinuteFromNow() }, keyRevision: 1 }) }, ready: async (page) => { await text(page, '48312290'); } },
  { id: 'clients-no-code', what: 'No pairing code showing.', role: 'server', hash: 'clients', options: { answers: answers('server/enrollment', { code: null, keyRevision: 1 }) }, ready: async (page) => { await text(page, 'New pairing code'); } },
  {
    id: 'clients-code-pending', what: 'A new pairing code has been asked for.', role: 'server', hash: 'clients', options: { answers: answers('server/enrollment', { code: null, keyRevision: 1 }) },
    act: async (page) => { await page.getByRole('button', { name: 'Show a new pairing code' }).click(); },
    ready: async (page) => { await page.waitForTimeout(400); },
  },
  { id: 'clients-empty', what: 'No computer is linked, nobody is asking.', role: 'server', hash: 'clients', options: { answers: { ...withClients([]), ...answers('server/pairing', { requests: [], budgetExhausted: false, window: { open: false, alwaysOpen: false, until: null } }) } }, ready: async (page) => { await text(page, /No computer is linked yet/); } },
  {
    id: 'clients-budget', what: 'New pairing requests are being turned away.', role: 'server', hash: 'clients',
    options: { answers: answers('server/pairing', { requests: [], budgetExhausted: true, window: { open: true, alwaysOpen: false, until: aMinuteFromNow() } }) },
    ready: async (page) => { await text(page, 'New requests are being turned away'); },
  },
  {
    id: 'clients-always-open', what: 'The server takes pairing requests at any time.', role: 'server', hash: 'clients',
    options: { answers: answers('server/pairing', { ...wire('pairing-overview'), window: { open: true, alwaysOpen: true, until: null } }) },
    ready: async (page) => { await text(page, 'takes pairing requests at any time'); },
  },
  {
    id: 'clients-approve-typed', what: 'A waiting request with its eight digits typed.', role: 'server', hash: 'clients',
    act: async (page) => { await page.getByLabel(/^Code shown on/).first().fill('12345678'); },
    ready: async (page) => { await expect(page.getByRole('button', { name: 'Approve' }).first()).toBeEnabled(); },
  },
  { id: 'clients-revoke-dialog', what: 'Revoking a computer: the confirmation.', role: 'server', hash: 'clients', act: async (page) => { await page.getByRole('button', { name: 'Revoke bench-laptop' }).click(); }, ready: async (page) => { await expect(page.getByRole('alertdialog', { name: /Revoke/ })).toBeVisible(); } },
  { id: 'clients-rotate-dialog', what: 'Rotating a computer\'s secret: the confirmation.', role: 'server', hash: 'clients', act: async (page) => { await page.getByRole('button', { name: 'Rotate the secret of bench-laptop' }).click(); }, ready: async (page) => { await expect(page.getByRole('alertdialog')).toBeVisible(); } },
  {
    id: 'clients-rotate-result', what: 'The new secret of a computer, shown once.', role: 'server', hash: 'clients',
    prepare: (page) => reply(page, 'POST', '**/api/v1/server/clients/client-1/secret', 200, wire('client-secret')),
    act: async (page) => { await page.getByRole('button', { name: 'Rotate the secret of bench-laptop' }).click(); await page.getByRole('button', { name: 'Rotate secret' }).click(); },
    ready: async (page) => { await text(page, /New secret for bench-laptop/); },
  },
  {
    id: 'clients-rotate-failed', what: 'Rotating a secret failed.', role: 'server', hash: 'clients', logsErrors: true,
    prepare: (page) => refuse(page, 'POST', '**/api/v1/server/clients/client-1/secret', 500, { title: 'Server error', status: 500, detail: 'The secrets file could not be written.' }),
    act: async (page) => { await page.getByRole('button', { name: 'Rotate the secret of bench-laptop' }).click(); await page.getByRole('button', { name: 'Rotate secret' }).click(); },
    ready: async (page) => { await text(page, 'Not rotated'); },
  },
  { id: 'clients-add-dialog', what: 'Adding a computer by hand.', role: 'server', hash: 'clients', act: async (page) => { await page.getByRole('button', { name: 'Add by hand' }).click(); }, ready: async (page) => { await heading(page, 'Add a client by hand'); } },
  {
    id: 'clients-add-result', what: 'A computer added by hand: its ID and secret.', role: 'server', hash: 'clients',
    prepare: (page) => reply(page, 'POST', '**/api/v1/server/clients', 201, wire('new-client')),
    act: async (page) => { await page.getByRole('button', { name: 'Add by hand' }).click(); await page.getByLabel('Name').fill('new-client'); await page.getByRole('button', { name: 'Add client' }).click(); },
    ready: async (page) => { await text(page, 'new-client is added'); },
  },
  {
    id: 'clients-key', what: 'The server key, shown.', role: 'server', hash: 'clients',
    prepare: (page) => page.route('**/api/v1/server/enrollment/key', (route) => route.fulfill({ json: wire('enrollment-key') })),
    act: async (page) => { await page.getByRole('button', { name: 'Show' }).first().click(); },
    ready: async (page) => { await text(page, /Server key/); await page.waitForTimeout(300); },
  },
  { id: 'clients-rotate-key-dialog', what: 'Rotating the server key: the confirmation.', role: 'server', hash: 'clients', act: async (page) => { await page.getByRole('button', { name: 'Rotate key' }).click(); }, ready: async (page) => { await expect(page.getByRole('alertdialog', { name: /Rotate the server key/ })).toBeVisible(); } },
  { id: 'clients-locked', what: 'Clients while the link is down.', role: 'server', hash: 'clients', options: { link: 'reconnecting' }, ready: async (page) => { await text(page, 'Reconnecting to the Koppel service'); } },
  {
    id: 'clients-long', what: 'Clients with very long names.', role: 'server', hash: 'clients',
    options: { answers: withClients([{ ...wire('clients')[0], name: long }, { ...wire('clients')[1], name: long + 'b' }]) },
    ready: async (page) => { await text(page, 'Computers'); },
  },
  { id: 'client', what: 'One computer.', role: 'server', hash: 'clients', act: async (page) => { await page.getByText('bench-laptop').first().click(); }, ready: async (page) => { await text(page, 'Client ID'); await text(page, 'Secret'); } },
  { id: 'client-no-secret', what: 'A computer added before secrets were kept.', role: 'server', hash: 'clients', act: async (page) => { await page.getByText('old-client').first().click(); }, ready: async (page) => { await text(page, /Added before Koppel kept secrets/); } },
  {
    id: 'client-secret', what: 'A computer\'s secret, asked for and shown.', role: 'server', hash: 'clients',
    prepare: (page) => page.route('**/api/v1/server/clients/client-1/secret', (route) => (route.request().method() === 'GET' ? route.fulfill({ json: wire('client-secret') }) : route.fallback())),
    act: async (page) => { await page.getByText('bench-laptop').first().click(); await page.getByRole('button', { name: 'Show' }).first().click(); },
    ready: async (page) => { await page.waitForTimeout(400); },
  },
  {
    id: 'client-gone', what: 'The opened computer is revoked.', role: 'server', hash: 'clients',
    act: async (page) => { await page.getByText('bench-laptop').first().click(); await text(page, 'Client ID'); await page.evaluate((d) => (window as unknown as { __push: (n: string, x: unknown) => void }).__push('ClientsChanged', d), [wire('clients')[1]]); },
    // The page goes back to the list when the computer it shows is gone (a device gets "Device not found").
    ready: async (page) => { await text(page, 'Computers'); await expect(page.getByText('Client ID', { exact: true })).toHaveCount(0); },
  },

  // ------------------------------------------------------------------ activity
  { id: 'activity', what: 'Activity with the fixture event.', role: 'both', hash: 'activity', ready: async (page) => { await text(page, /could not be attached/); } },
  { id: 'activity-empty', what: 'Activity with no event.', role: 'both', hash: 'activity', options: { answers: answers('activity', []) }, ready: async (page) => { await text(page, 'No events yet'); } },
  {
    id: 'activity-many', what: 'Activity over several days, succeeded and rejected, one with very long text.', role: 'both', hash: 'activity',
    options: {
      answers: answers('activity', [
        wire('activity-event'),
        { ...wire('activity-event'), id: '0000019b8a1c2d3e-0002', outcome: 'succeeded', detail: null, summary: 'USB Receiver was shared.', timestampUtc: '2025-12-30T10:00:00Z' },
        { ...wire('activity-event'), id: '0000019b8a1c2d3e-0003', summary: long, detail: long, timestampUtc: '2025-12-29T10:00:00Z' },
      ]),
    },
    ready: async (page) => { await text(page, 'was shared'); },
  },
  { id: 'activity-no-match', what: 'A search that matches no event.', role: 'both', hash: 'activity', act: async (page) => { await page.getByRole('searchbox').first().fill('zzzz'); }, ready: async (page) => { await text(page, /No events match/); } },

  // ------------------------------------------------------------------ system check
  { id: 'check', what: 'System check with a failing check.', role: 'both', hash: 'check', options: { answers: answers('system/checks', checks) }, ready: async (page) => { await text(page, 'Needs a change'); } },
  { id: 'check-ready', what: 'System check, all ready.', role: 'both', hash: 'check', ready: async (page) => { await text(page, 'All ready'); } },
  { id: 'check-client', what: 'System check on a client: only the attach check.', role: 'client', hash: 'check', options: { answers: answers('system/checks', checks) }, ready: async (page) => { await text(page, /vhci-hcd/); } },
  { id: 'check-failed', what: 'System check cannot reach the service.', role: 'both', hash: 'check', options: { answers: answers('system/checks', null) }, logsErrors: true, prepare: (page) => page.route('**/api/v1/system/checks', (route) => route.fulfill({ status: 500, body: '' })), ready: async (page) => { await heading(page, 'Could not reach Koppel'); } },

  // ------------------------------------------------------------------ connection
  { id: 'connection-first', what: 'Connection of a client that is not paired: the form.', role: 'client', hash: 'connection', options: { answers: answers('client/servers', { servers: [] }) }, ready: async (page) => { await text(page, 'Pair with a server'); } },
  { id: 'connection-servers', what: 'Connection of a client with a server, the panel closed.', role: 'client', hash: 'connection', ready: async (page) => { await text(page, 'Servers'); } },
  {
    id: 'connection-link-panel', what: 'The settings panel of a server (a sheet on a phone).', role: 'client', hash: 'connection',
    act: async (page) => { await page.getByRole('button', { name: /Settings for lab/ }).first().click(); },
    ready: async (page) => { await text(page, 'Retry every'); },
  },
  {
    id: 'connection-link-edited', what: 'The panel with a value edited: Revert and Apply enabled.', role: 'client', hash: 'connection',
    act: async (page) => { await page.getByRole('button', { name: /Settings for lab/ }).first().click(); const field = page.getByLabel('Retry every'); await field.fill('45'); await field.blur(); },
    ready: async (page) => { await expect(page.getByRole('button', { name: 'Apply' })).toBeEnabled(); },
  },
  {
    id: 'connection-reconnect-off', what: 'The panel with Reconnect off: Retry every is disabled with its hint.', role: 'client', hash: 'connection',
    options: { answers: answers('client/servers', { servers: [{ ...wire('servers').servers[0], reconnect: false }] }) },
    act: async (page) => { await page.getByRole('button', { name: /Settings for lab/ }).first().click(); },
    ready: async (page) => { await text(page, 'Turn Reconnect on to retry.'); },
  },
  {
    id: 'connection-not-saved', what: 'A change to a server was not saved.', role: 'client', hash: 'connection', logsErrors: true,
    prepare: (page) => refuse(page, 'PATCH', '**/api/v1/client/servers/server-1', 400, { title: 'Bad request', status: 400, detail: 'Retry every must be at least 1 second.' }),
    act: async (page) => { await page.getByRole('button', { name: /Settings for lab/ }).first().click(); const field = page.getByLabel('Retry every'); await field.fill('45'); await field.blur(); await click(page, 'Apply'); },
    ready: async (page) => {
      await text(page, /is not saved/);
      // On a phone the panel is a sheet over that, so the same failure must show inside the panel.
      if ((page.viewportSize()?.width ?? 1000) < 800) await expect(page.getByText('Not saved', { exact: true })).toBeVisible();
    },
  },
  {
    id: 'connection-add', what: 'Adding another server: the form, with a server found on the network.', role: 'client', hash: 'connection',
    act: async (page) => { await page.getByRole('button', { name: 'Add a server' }).click(); },
    ready: async (page) => { await text(page, 'Found on this network'); },
  },
  {
    id: 'connection-add-key', what: 'The add form switched to the server key.', role: 'client', hash: 'connection',
    act: async (page) => { await page.getByRole('button', { name: 'Add a server' }).click(); await page.getByRole('button', { name: 'Use the server key' }).click(); },
    ready: async (page) => { await expect(page.getByLabel('Server key')).toBeVisible(); },
  },
  {
    id: 'connection-add-refused', what: 'The add form after the service refuses the address.', role: 'client', hash: 'connection', logsErrors: true,
    prepare: (page) => refuse(page, 'POST', '**/api/v1/client/servers', 409, { title: 'Conflict', status: 409, code: 'already_added', detail: 'This computer already has that server.' }),
    act: async (page) => { await page.getByRole('button', { name: 'Add a server' }).click(); await page.getByLabel('Server address').fill('nowhere'); await page.getByLabel('Pairing code').fill('12345678'); await click(page, 'Add server'); },
    ready: async (page) => { await text(page, /already has that server/); },
  },
  {
    id: 'connection-add-wrong-code', what: 'The add form after the service refuses the pairing code.', role: 'client', hash: 'connection', logsErrors: true,
    prepare: (page) => refuse(page, 'POST', '**/api/v1/client/servers', 403, { title: 'Forbidden', status: 403, code: 'wrong_code', detail: 'That pairing code is not right, or it has expired.' }),
    act: async (page) => { await page.getByRole('button', { name: 'Add a server' }).click(); await page.getByLabel('Server address').fill('lab-rack-3:5241'); await page.getByLabel('Pairing code').fill('12345678'); await click(page, 'Add server'); },
    ready: async (page) => { await text(page, /pairing code is not right/); },
  },
  {
    id: 'connection-two-servers', what: 'Two servers with different link states.', role: 'client', hash: 'connection',
    options: { answers: { ...answers('client/servers', { servers: [...wire('servers').servers, { ...wire('servers').servers[0], id: 'server-2', name: null, serverUrl: `https://${'cellar-with-a-long-host-name.'.repeat(3)}example.net:5241`, linkedWith: 'key', reconnect: false }] }), ...answers('client/devices', clientView(clientDevices.servers[0], awayServer({ link: 'error', linkMessage: 'Connection refused.' }))) } },
    ready: async (page) => { await text(page, 'Cannot reach'); },
  },
  {
    id: 'connection-test-failed', what: 'The connection test: a toast with the certificate mismatch.', role: 'client', hash: 'connection',
    prepare: (page) => reply(page, 'POST', '**/api/v1/client/servers/server-1/test', 200, wire('connection-test')),
    act: async (page) => { await page.getByRole('button', { name: /Test the connection to lab/ }).click(); },
    ready: async (page) => { await text(page, 'The connection test failed'); },
  },
  {
    id: 'connection-test-ok', what: 'The connection test: a toast that the server answers.', role: 'client', hash: 'connection',
    prepare: (page) => reply(page, 'POST', '**/api/v1/client/servers/server-1/test', 200, { ok: true, outcome: 'ok', message: 'The server answered in 12 ms.' }),
    act: async (page) => { await page.getByRole('button', { name: /Test the connection to lab/ }).click(); },
    ready: async (page) => { await text(page, 'lab answers'); },
  },
  { id: 'connection-remove-dialog', what: 'Removing a server: the confirmation.', role: 'client', hash: 'connection', act: async (page) => { await page.getByRole('button', { name: /Remove lab/ }).click(); }, ready: async (page) => { await expect(page.getByRole('alertdialog', { name: /Remove lab/ })).toBeVisible(); } },
  { id: 'connection-pending', what: 'A server is being removed.', role: 'client', hash: 'connection', act: async (page) => { await page.getByRole('button', { name: /Remove lab/ }).click(); await page.getByRole('button', { name: 'Remove', exact: true }).last().click(); }, ready: async (page) => { await text(page, 'Removing'); } },
  { id: 'connection-server', what: 'Connection of a server: its own settings panel (a sheet on a phone).', role: 'server', hash: 'connection', act: async (page) => { await page.getByRole('button', { name: /Settings for bench-pc/ }).first().click(); }, ready: async (page) => { await text(page, 'Accept from'); } },
  {
    id: 'connection-port-fixed', what: 'The port is fixed by a startup option.', role: 'server', hash: 'connection', options: { info: { networkPortFixed: true } },
    act: async (page) => { await page.getByRole('button', { name: /Settings for bench-pc/ }).first().click(); },
    ready: async (page) => { await text(page, 'A startup option sets the port'); },
  },
  {
    id: 'connection-port-restart', what: 'A port saved that is not the one in use: applies after a restart.', role: 'server', hash: 'connection', options: { settings: { networkPort: 8443 } },
    act: async (page) => { await page.getByRole('button', { name: /Settings for bench-pc/ }).first().click(); },
    ready: async (page) => { await text(page, 'Applies after Koppel restarts'); },
  },
  {
    id: 'connection-both', what: 'Connection of a server and client: the servers table and this server.', role: 'both', hash: 'connection',
    ready: async (page) => { await text(page, 'This server'); await text(page, 'Servers'); },
  },
  { id: 'connection-locked', what: 'Connection while the link is down.', role: 'both', hash: 'connection', options: { link: 'reconnecting' }, ready: async (page) => { await text(page, 'Reconnecting to the Koppel service'); } },

  // ------------------------------------------------------------------ security
  { id: 'security', what: 'Security of a server: the certificate with a new one prepared, and sign-in.', role: 'server', hash: 'security', ready: async (page) => { await text(page, 'Server certificate'); await text(page, 'Admin password'); } },
  {
    id: 'security-no-next', what: 'Security with no new certificate prepared.', role: 'server', hash: 'security',
    options: { answers: answers('server/keys', { ...wire('keys-view'), nextFingerprint: null, nextPreparedAt: null }) },
    ready: async (page) => { await expect(page.getByRole('button', { name: 'Prepare a new certificate' })).toBeVisible(); },
  },
  {
    id: 'security-prepare-pending', what: 'A new certificate is being prepared.', role: 'server', hash: 'security',
    options: { answers: answers('server/keys', { ...wire('keys-view'), nextFingerprint: null, nextPreparedAt: null }) },
    act: async (page) => { await page.getByRole('button', { name: 'Prepare a new certificate' }).click(); },
    ready: async (page) => { await text(page, 'Preparing'); },
  },
  { id: 'security-switch-dialog', what: 'Switching certificate while a computer lacks the new one: the confirmation.', role: 'server', hash: 'security', act: async (page) => { await page.getByRole('button', { name: 'Switch to the new certificate' }).click(); }, ready: async (page) => { await expect(page.getByRole('alertdialog', { name: /Switch to the new certificate/ })).toBeVisible(); } },
  { id: 'security-discard-dialog', what: 'Discarding the new certificate: the confirmation.', role: 'server', hash: 'security', act: async (page) => { await page.getByRole('button', { name: 'Discard the new certificate' }).click(); }, ready: async (page) => { await expect(page.getByRole('alertdialog', { name: /Discard the new certificate/ })).toBeVisible(); } },
  {
    id: 'security-failed', what: 'A certificate step failed.', role: 'server', hash: 'security', logsErrors: true,
    prepare: (page) => refuse(page, 'POST', '**/api/v1/server/keys/next-certificate', 500, { title: 'Server error', status: 500, detail: 'The key store is read only.' }),
    options: { answers: answers('server/keys', { ...wire('keys-view'), nextFingerprint: null, nextPreparedAt: null }) },
    act: async (page) => { await page.getByRole('button', { name: 'Prepare a new certificate' }).click(); },
    ready: async (page) => { await text(page, 'Not done'); },
  },
  { id: 'security-password-dialog', what: 'Changing the admin password.', role: 'server', hash: 'security', act: async (page) => { await page.getByRole('button', { name: 'Change password' }).click(); }, ready: async (page) => { await expect(page.getByRole('dialog')).toBeVisible(); } },
  { id: 'security-client', what: 'Security of a client: sign-in only.', role: 'client', hash: 'security', ready: async (page) => { await text(page, 'Admin password'); } },
  { id: 'security-locked', what: 'Security while the link is down.', role: 'server', hash: 'security', options: { link: 'reconnecting' }, ready: async (page) => { await text(page, 'Reconnecting to the Koppel service'); } },

  // ------------------------------------------------------------------ settings
  { id: 'settings-server', what: 'Settings of a server.', role: 'server', hash: 'settings', ready: async (page) => { await text(page, 'Appearance'); } },
  { id: 'settings-client', what: 'Settings of a client.', role: 'client', hash: 'settings', ready: async (page) => { await text(page, 'Appearance'); } },
  { id: 'settings-both-tray', what: 'Settings of a server and client with the notification choices (a tray is there).', role: 'both', hash: 'settings', options: { info: { tray: true } }, ready: async (page) => { await text(page, 'Notify me when'); } },
  { id: 'settings-unknown-role', what: 'Settings with a role this version does not know.', role: 'server', hash: 'settings', options: { settings: { role: 'relay' } }, ready: async (page) => { await text(page, /does not know the role/); } },
  {
    id: 'settings-not-saved', what: 'A setting was refused.', role: 'server', hash: 'settings', logsErrors: true,
    prepare: (page) => refuse(page, 'PATCH', '**/api/v1/settings', 400, { title: 'Bad request', status: 400, detail: 'The config file is read only.' }),
    act: async (page) => { await page.getByRole('switch', { name: /Share saved devices on startup/ }).click({ force: true }); },
    ready: async (page) => { await text(page, 'Not saved'); },
  },
  { id: 'settings-locked', what: 'Settings while the link is down.', role: 'server', hash: 'settings', options: { link: 'reconnecting' }, ready: async (page) => { await text(page, 'Reconnecting to the Koppel service'); } },
];
