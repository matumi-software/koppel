using System.Collections.Concurrent;
using System.Net;
using System.Net.Sockets;
using System.Net.WebSockets;
using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Core.Configuration;
using Koppel.Core.Usb;
using Koppel.Settings;
using Skakel.Client;

namespace Koppel.Usb;

/// <summary>A remote device as the client's UI sees it.</summary>
/// <param name="Port">The port this machine's kernel or driver attached it on; null for another gateway attach.</param>
/// <param name="AttachedVia">Where a gateway attach came from, such as <c>172.17.0.1</c>.</param>
public sealed record ClientDeviceView(DeviceView Device, bool AttachedHere, int? Port, DateTimeOffset? AttachedSince, string? Fault, string? AttachedVia = null);

/// <summary>What the client's UI shows: the link to the server and the devices it shares.</summary>
/// <param name="Link"><c>unconfigured</c>, <c>connecting</c>, <c>connected</c> or <c>error</c>.</param>
/// <param name="Gateway">True while the plain USB/IP gateway listens for a driver Koppel does not drive itself
/// (Koppel in a container on Windows), so the UI can show the command that attaches through it.</param>
/// <param name="GatewayFault">Why the gateway is not listening although it should be, such as a port that is taken.</param>
public sealed record ClientDevices(string Link, string? LinkMessage, IReadOnlyList<ClientDeviceView> Devices, string? Unavailable, bool Gateway = false, string? GatewayFault = null);

/// <summary>
/// The devices this client attached, kept in the plain <c>attached.json</c>. The client attaches them
/// again whenever the server offers them and they are not attached here: after a restart, an upgrade,
/// a dropped link, or the device being plugged back in on the server. Detaching forgets the device.
/// </summary>
/// <param name="ServerFingerprint">The server the bus IDs belong to. Another server starts a new list; the same server
/// switching to its next certificate carries the list over.</param>
/// <param name="VidPids">The VID:PID each bus ID had when it was attached. A bus ID is a port on the
/// server, so a different device plugged into that port is not attached without asking.</param>
internal sealed record AttachedDevices(string? ServerFingerprint, IReadOnlyList<string> BusIds, IReadOnlyDictionary<string, string>? VidPids = null)
{
    public static AttachedDevices Empty { get; } = new(null, []);

    public bool Expects(DeviceView device) =>
        BusIds.Contains(device.BusId) && (VidPids?.GetValueOrDefault(device.BusId) is not { } vidPid || vidPid == device.VidPid);
}

/// <summary>
/// A device on a server. A bus ID is only a port on one server, so attachments are told apart by the
/// server as well: the same bus ID on another server is another device. The server is named by the client
/// ID it issued, which stays the same when the server switches to a new certificate.
/// </summary>
internal readonly record struct AttachKey(string Server, string BusId);

/// <summary>
/// Client role: keeps a Skakel link to the configured server for its device list, and attaches and
/// detaches devices. An attach opens a <c>usb</c> stream on the link (a WebSocket over TLS, pinned to the
/// server's certificate, with the link's token), imports the device over USB/IP, then hands one end of a
/// loopback pair to the kernel's virtual host controller and relays the other end through the stream.
/// </summary>
/// <remarks>
/// The link keeps the hub connected, renews it before its token runs out, follows the server to another
/// address on the local network, and learns the server's next certificate. What it learns is saved in
/// <c>connection.json</c> here, and the attached devices carry on through all of it.
/// </remarks>
internal sealed partial class UsbClient(
    KoppelPaths paths,
    ConnectionStore connection,
    SettingsStore settings,
    SkakelBrowser browser,
    IUsbAttacher attacher,
    IUsbIpDriver driver,
    TimeProvider time,
    IActivityRecorder activity,
    IConfiguration configuration,
    ILoggerFactory loggers,
    ILogger<UsbClient> logger) : BackgroundService
{
    /// <summary>How often a remembered device that is not attached yet is tried again.</summary>
    private static readonly TimeSpan RetryInterval = TimeSpan.FromSeconds(5);

    /// <summary>
    /// One deadline for everything before USB traffic flows: the token, connecting, TLS, the WebSocket, the
    /// import reply and the device record. A server that accepts and then says nothing ends here.
    /// </summary>
    private readonly TimeSpan _handshakeTimeout = TimeSpan.FromSeconds(configuration.GetValue("Usb:AttachTimeoutSeconds", 20));

    private readonly JsonConfigFile<AttachedDevices> _remembered = new(
        Path.Combine(paths.ConfigDirectory, "attached.json"), AttachedDevices.Empty, KoppelJson.Files, logger, startWithDefaultsWhenUnusable: true);

    /// <summary>Why <c>attached.json</c> is not being used, or null. Koppel started with nothing remembered.</summary>
    public string? RememberedProblem => _remembered.Problem;

    private readonly BackgroundTasks _tasks = new(logger);
    private readonly ConcurrentDictionary<AttachKey, Attachment> _attachments = new();
    private readonly ConcurrentDictionary<AttachKey, bool> _attaching = new();
    private readonly ConcurrentDictionary<AttachKey, PendingImport> _pending = new();
    private readonly ConcurrentDictionary<AttachKey, CancellationTokenSource> _inFlight = new();
    private readonly ConcurrentDictionary<string, bool> _detaching = new();
    private readonly ConcurrentDictionary<AttachKey, DriverAttachment> _driverPorts = new();
    private readonly ConcurrentDictionary<string, DateTimeOffset> _lastTry = new();
    private readonly ConcurrentDictionary<string, string> _faults = new();
    private readonly object _gate = new();
    private RemoteDevices _remote = new([]);

    /// <summary>True once the server sent its device list on this link, so its USB data plane is worth trying.</summary>
    private volatile bool _listed;
    private string _link = "unconfigured";
    private string? _linkMessage;
    private CancellationTokenSource _epoch = new();
    private RemoteLink? _remoteLink;
    private (bool Client, ClientConnection Connection) _applied;
    private Task _releasing = Task.CompletedTask;
    private int _attachStarts;
    private int _changesApplied;
    private CancellationToken _stopping;
    private int? _gatewayPort;
    private string? _gatewayFault;

    public event Func<ClientDevices, Task>? Changed;

    /// <summary>How many attaches actually started, for tests that prove a repeated request starts none.</summary>
    internal int AttachStarts => Volatile.Read(ref _attachStarts);

    /// <summary>The port the gateway listens on, or null while it does not.</summary>
    internal int? GatewayPort => _gatewayPort;

    /// <summary>How many role or connection changes were applied, for tests that prove an unrelated setting applies none.</summary>
    internal int ChangesApplied => Volatile.Read(ref _changesApplied);

    public ClientDevices Current
    {
        get
        {
            return new(
                _link,
                _linkMessage,
                [.. _remote.Devices.Select(d =>
                {
                    var a = _attachments.GetValueOrDefault(KeyOf(d.BusId));
                    return new ClientDeviceView(d, a is not null, a?.Port, a?.Since, _faults.GetValueOrDefault(d.BusId), a?.Via);
                })],
                UsesDriver ? null : _gatewayFault ?? attacher.Unavailable,
                _gatewayPort is not null && !UsesDriver,
                _gatewayFault);
        }
    }

    /// <summary>
    /// True when attaching goes through a driver on this machine (usbip-win2 on Windows) by way of the
    /// gateway, rather than through this kernel's virtual host controller.
    /// </summary>
    private bool UsesDriver => _gatewayPort is not null && driver.Unavailable is null;

    /// <summary>
    /// Called by <see cref="UsbIpGateway"/> with its port when it starts listening, and null when it
    /// stops. <paramref name="fault"/> says why it is not listening although it should be.
    /// </summary>
    public Task SetGatewayAsync(int? port, string? fault = null)
    {
        _gatewayPort = port;
        _gatewayFault = fault;
        return RaiseAsync();
    }

    public override async Task StartAsync(CancellationToken cancellationToken)
    {
        await _remembered.StartAsync(cancellationToken);
        await base.StartAsync(cancellationToken);
    }

    /// <summary>
    /// Stops the link and, on Windows, detaches every device Koppel attached through the driver. The
    /// driver would otherwise keep trying to attach them to a gateway port that nobody holds any more,
    /// and anything that bound that port would get them. The devices stay remembered, so they attach
    /// again when Koppel starts.
    /// </summary>
    public override async Task StopAsync(CancellationToken cancellationToken)
    {
        _remembered.Stop();
        await EndEpochAsync();
        await base.StopAsync(cancellationToken);
        // Relays end on their own once told to, so shutdown does not wait for them. The devices stay remembered.
        foreach (var attachment in _attachments.Values)
        {
            await StopRelayAsync(attachment);
        }

        foreach (var key in _driverPorts.Keys)
        {
            try
            {
                await ReleaseDriverAsync(key, cancellationToken);
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                LogDetachFailed(key.BusId, RemoteFailure.Describe(ex));
            }
        }

        await _tasks.WaitAsync(TimeSpan.FromSeconds(5));
    }

    public override void Dispose()
    {
        _remembered.Dispose();
        base.Dispose();
    }

    /// <summary>The link to the saved server, or null while there is none. For tests that check it is kept.</summary>
    internal RemoteLink? Remote => Volatile.Read(ref _remoteLink);

    /// <summary>
    /// Remembers the device, then starts attaching it in the background. The outcome reaches the UI
    /// through <see cref="Changed"/>. A device that is already attached or being attached is left alone.
    /// </summary>
    public async Task BeginAttachAsync(string busId)
    {
        UsbBusId.Require(busId);
        await RememberAsync(busId, attached: true);
        _faults.TryRemove(busId, out _);
        StartAttach(busId, automatic: false);
    }

    /// <summary>
    /// Releases the device first and forgets it after, so a failed save of <c>attached.json</c> is a fault
    /// on the device and never leaves it attached. Until it is forgotten the retry skips it.
    /// </summary>
    public async Task DetachAsync(string busId)
    {
        _detaching[busId] = true;
        try
        {
            _faults.TryRemove(busId, out _);
            var key = KeyOf(busId);
            // An attach still on its way notices this once the driver returns, and releases what it made.
            CancelInFlight(key);
            _attachments.TryRemove(key, out var attachment);
            if (attachment is not null || _driverPorts.ContainsKey(key))
            {
                await ReleaseAsync(key, attachment, CancellationToken.None);
            }

            try
            {
                await RememberAsync(busId, attached: false);
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                _faults[busId] = $"The device was detached, but Koppel could not save that, so it may attach again after a restart: {ex.Message}";
                LogDetachFailed(busId, ex.Message);
            }
        }
        finally
        {
            _detaching.TryRemove(busId, out _);
        }

        await RaiseAsync();
    }

    private void CancelInFlight(AttachKey key)
    {
        if (_inFlight.TryGetValue(key, out var attach))
        {
            try
            {
                attach.Cancel();
            }
            catch (ObjectDisposedException)
            {
                // The attach finished first.
            }
        }
    }

    /// <summary>
    /// Undoes an attach: the driver's or the kernel's port, then the relay. A driver attach is undone by
    /// the driver, which would otherwise attach it again. Another gateway attach has no port here;
    /// stopping its relay unplugs it on the other side.
    /// </summary>
    private async Task ReleaseAsync(AttachKey key, Attachment? attachment, CancellationToken cancellationToken)
    {
        try
        {
            var detached = await ReleaseDriverAsync(key, cancellationToken);
            if (!detached && attachment is { Via: null, Driver: false, Port: { } port })
            {
                attacher.Detach(port);
                detached = true;
            }

            if (detached)
            {
                await activity.RecordAsync(Device(key.BusId), ActivityReason.Detached, ActivityOutcome.Succeeded, $"{NameOf(key.BusId)} was detached");
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            _faults[key.BusId] = RemoteFailure.Describe(ex);
            LogDetachFailed(key.BusId, ex.Message);
        }

        if (attachment is not null)
        {
            await StopRelayAsync(attachment);
        }
    }

    private static async Task StopRelayAsync(Attachment attachment)
    {
        try
        {
            await attachment.Stop.CancelAsync();
        }
        catch (ObjectDisposedException)
        {
            // The session ended by itself first, as it does once a driver lets go of the device.
        }
    }

    /// <summary>Stops the driver's attempts to attach the device and detaches its port. False when the driver holds none.</summary>
    private async Task<bool> ReleaseDriverAsync(AttachKey key, CancellationToken cancellationToken)
    {
        // Taking the entry first means a detach and an attach that finds itself unwanted never both release it.
        if (!_driverPorts.TryRemove(key, out var held))
        {
            return false;
        }

        try
        {
            await driver.DetachAsync(IPAddress.Loopback.ToString(), held.GatewayPort, key.BusId, held.DriverPort, cancellationToken);
        }
        catch
        {
            // Still attached, so the next release tries again.
            _driverPorts.TryAdd(key, held);
            throw;
        }

        return true;
    }

    /// <summary>Detaches and stops every attachment, without forgetting any of them.</summary>
    private async Task ReleaseAllAsync()
    {
        foreach (var (key, attachment) in _attachments.ToArray())
        {
            if (_attachments.TryRemove(new KeyValuePair<AttachKey, Attachment>(key, attachment)))
            {
                await ReleaseAsync(key, attachment, CancellationToken.None);
            }
        }

        foreach (var key in _driverPorts.Keys)
        {
            await ReleaseAsync(key, null, CancellationToken.None);
        }
    }

    private (bool Client, ClientConnection Connection) Settled() => (settings.Current.Role != KoppelRole.Server, connection.Current);

    /// <summary>What the next attach waits for: the release that a role or connection change started.</summary>
    private Task Releasing()
    {
        lock (_gate)
        {
            return _releasing;
        }
    }

    /// <summary>
    /// A new server, new credentials, or a role that is no longer Client or Both ends every attachment
    /// first, so nothing stays attached to a server the user moved away from, and then restarts the link.
    /// A change that touches neither (the listen port, say) leaves the link alone, and so does the same
    /// server announcing or switching to its next certificate (<see cref="ClientConnection.Continues"/>).
    /// The same server at another address keeps the attachments, and the link too when the link followed
    /// the server there itself; an address the person typed starts the link again there.
    /// The release runs in the background, because the driver's own command may take 30 seconds and the
    /// settings update, and with it the UI's confirmation, waits for this handler. An attach that starts
    /// meanwhile waits for the release.
    /// </summary>
    private async Task OnChangedAsync()
    {
        var previous = Task.CompletedTask;
        TaskCompletionSource? done = null;
        ClientConnection was;
        lock (_gate)
        {
            // Read inside the lock, so two changes at once cannot store each other's stale state.
            var now = Settled();
            if (_applied == now)
            {
                return;
            }

            was = _applied.Connection;
            var sameServer = _applied.Client == now.Client && now.Connection.Continues(was);
            _applied = now;
            if (!sameServer)
            {
                previous = _releasing;
                done = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                _releasing = done.Task;
            }
        }

        if (done is null)
        {
            // The same server: the attachments carry on, and so does the link unless it is at another address.
            await ApplyLinkAsync();
            await CarryOverRememberedAsync(was);
            return;
        }

        Interlocked.Increment(ref _changesApplied);
        try
        {
            await EndEpochAsync();
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            LogChangeFailed(ex);
        }

        _faults.Clear();
        _lastTry.Clear();
        var before = _remote;
        _tasks.Run("release after a change", async () =>
        {
            try
            {
                await previous;
                await ReleaseAllAsync();
                // The new link may have listed its devices by now; only the old list is dropped.
                if (ReferenceEquals(Interlocked.CompareExchange(ref _remote, new RemoteDevices([]), before), before))
                {
                    _listed = false;
                }
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                LogChangeFailed(ex);
            }
            finally
            {
                done.SetResult();
            }

            await RaiseAsync();
        });

        await ApplyLinkAsync();
        await RaiseAsync();
    }

    /// <summary>
    /// The server switched to its next certificate: the devices remembered for it stay remembered under the
    /// new fingerprint, so they attach again after a restart. Nothing to do when only the next one changed.
    /// </summary>
    private async Task CarryOverRememberedAsync(ClientConnection before)
    {
        var after = connection.Current.ServerFingerprint;
        if (after is null || after == before.ServerFingerprint)
        {
            return;
        }

        try
        {
            await _remembered.UpdateAsync(r => r.ServerFingerprint == before.ServerFingerprint ? r with { ServerFingerprint = after } : r, CancellationToken.None);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            LogChangeFailed(ex);
        }
    }

    /// <summary>Cancels in-flight attaches. They check this before and after they register.</summary>
    private async Task EndEpochAsync()
    {
        CancellationTokenSource ended;
        lock (_gate)
        {
            ended = _epoch;
            _epoch = new CancellationTokenSource();
        }

        await ended.CancelAsync();
    }

    private CancellationToken EpochToken()
    {
        lock (_gate)
        {
            return _epoch.Token;
        }
    }

    /// <summary>
    /// Keeps the link, while the role is Client or Both and the saved connection is complete, and tries the
    /// remembered devices again every few seconds while it is up.
    /// </summary>
    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        _stopping = stoppingToken;
        lock (_gate)
        {
            _applied = Settled();
        }

        Task Restart<T>(T _) => OnChangedAsync();

        connection.Changed += Restart;
        settings.Changed += Restart;
        try
        {
            await ApplyLinkAsync();

            // A remembered device that could not be attached yet, because it was in use or the server was
            // still starting, is tried again while the link is up.
            using var retry = new PeriodicTimer(RetryInterval, time);
            while (await retry.WaitForNextTickAsync(stoppingToken))
            {
                AttachRemembered();
            }
        }
        catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
        {
            // Stopping.
        }
        finally
        {
            connection.Changed -= Restart;
            settings.Changed -= Restart;
            ReplaceLink(null);
        }
    }

    /// <summary>
    /// Makes the link the saved connection needs, or none. The same server at the address the link is at
    /// keeps its link, so a certificate it switched to or an address it followed changes nothing here. A
    /// link that is replaced stops in the background: stopping waits for the link's own handlers, and one
    /// of them may be the save that led here.
    /// </summary>
    private async Task ApplyLinkAsync()
    {
        var c = connection.Current;
        var wanted = settings.Current.Role != KoppelRole.Server && c.IsComplete && !_stopping.IsCancellationRequested;
        RemoteLink? made = null;
        lock (_gate)
        {
            var current = _remoteLink;
            if (wanted && current is not null && c.Continues(current.Connection) && SameAddress(c, current.Link.Address))
            {
                current.Connection = c;
                return;
            }

            if (wanted)
            {
                made = NewLink(c);
            }

            // Current before it starts, so the first thing it reports is heard.
            ReplaceLinkLocked(made);
            made?.Link.Start();
        }

        if (made is null)
        {
            _remote = new([]);
            _listed = false;
            await SetLinkAsync("unconfigured", null);
        }
    }

    private static bool SameAddress(ClientConnection c, Uri address) =>
        string.Equals(c.GetAddress().Authority, address.Authority, StringComparison.OrdinalIgnoreCase);

    /// <summary>A link for <paramref name="c"/>, wired to this client. Not started.</summary>
    private RemoteLink NewLink(ClientConnection c)
    {
        var options = new SkakelClientOptions
        {
            LoggerFactory = loggers,
            TimeProvider = time,
            HubPath = RemoteHub.Path,
            Browser = configuration.GetValue("Discovery:Enabled", true) ? browser : null,
            ConfigureHubJson = KoppelJson.Configure,
        };
        var remote = new RemoteLink(c, new SkakelLink(c.GetAddress(), c.GetPins(), c.GetCredentials(), options));
        remote.Link.StatusChanged += status => OnLinkStatusAsync(remote, status);
        remote.Link.PinsChanged += pins => OnPinsChangedAsync(remote, pins);
        remote.Link.AddressChanged += to => OnLinkMovedAsync(remote, to);
        remote.Link.On<RemoteDevices>("DevicesChanged", devices => OnDevicesAsync(remote, devices));
        return remote;
    }

    private void ReplaceLink(RemoteLink? next)
    {
        lock (_gate)
        {
            ReplaceLinkLocked(next);
        }
    }

    private void ReplaceLinkLocked(RemoteLink? next)
    {
        var previous = _remoteLink;
        Volatile.Write(ref _remoteLink, next);
        if (previous is not null)
        {
            _tasks.Run("stop the old link", async () => await previous.Link.DisposeAsync());
        }
    }

    private bool IsCurrent(RemoteLink remote) => ReferenceEquals(Volatile.Read(ref _remoteLink), remote);

    /// <summary>The link to the saved server, or null while there is none.</summary>
    private RemoteLink? CurrentLink() => Volatile.Read(ref _remoteLink);

    /// <summary>
    /// Where the link stands, as the UI shows it. A revoke or a rotation on the server closes the link for
    /// good, with words that say to pair again. Renewing it before its token runs out is not a change.
    /// </summary>
    private async Task OnLinkStatusAsync(RemoteLink remote, SkakelLinkStatus status)
    {
        if (!IsCurrent(remote))
        {
            return;
        }

        switch (status.State)
        {
            case SkakelLinkState.Connecting:
                await SetLinkAsync("connecting", null);
                break;
            case SkakelLinkState.Connected:
                await SetLinkAsync("connected", null);
                AttachRemembered();
                break;
            case SkakelLinkState.Failed or SkakelLinkState.Closed:
                LogLinkFailed(status.Failure?.Code ?? "unknown");
                await SetLinkAsync("error", status.Failure?.Message ?? SkakelFailure.Of(SkakelFailureKind.ConnectionLost).Message);
                break;
        }
    }

    private async Task OnDevicesAsync(RemoteLink remote, RemoteDevices devices)
    {
        if (!IsCurrent(remote))
        {
            return;
        }

        _remote = Sanitize(devices);
        _listed = true;
        await RaiseAsync();
        AttachRemembered();
    }

    /// <summary>
    /// The server told the link of its next certificate, switched to it, or cancelled it. Saved in
    /// <c>connection.json</c> before the link tells the server it has it; a save that fails, or a saved
    /// connection that is another server's by now, throws, and the server says it again on the next link.
    /// </summary>
    private Task OnPinsChangedAsync(RemoteLink remote, SkakelPins pins) => connection.SavePinsAsync(remote.Connection, pins, CancellationToken.None);

    /// <summary>
    /// The server moved on the local network and the link followed it. The saved connection follows too,
    /// unless the person saved another one meanwhile, and the activity log says so.
    /// </summary>
    internal async Task OnLinkMovedAsync(RemoteLink remote, Uri to)
    {
        var was = remote.Connection.GetAddress().Authority;
        var url = to.GetLeftPart(UriPartial.Authority);
        if (!await connection.MoveServerAsync(remote.Connection, url, CancellationToken.None))
        {
            return;
        }

        LogMoved(was, to.Authority);
        await activity.RecordAsync(new EntityRef("link", to.Host, "link"), ActivityReason.ServerMoved, ActivityOutcome.Succeeded, $"Server moved from {was} to {to.Authority}");
    }

    /// <summary>
    /// What a server sends is only trusted once it is checked    /// <summary>
    /// What a server sends is only trusted once it is checked: a bus ID that is not one is dropped, so it
    /// never reaches the UI, a log line, or a command line.
    /// </summary>
    private RemoteDevices Sanitize(RemoteDevices? received)
    {
        var devices = received?.Devices ?? [];
        var valid = devices.Where(d => d is not null && UsbBusId.IsValid(d.BusId)).DistinctBy(d => d.BusId).ToList();
        if (valid.Count != devices.Count)
        {
            LogInvalidDevices(devices.Count - valid.Count);
        }

        return new RemoteDevices(valid);
    }

    /// <summary>
    /// Attaches in the background. A failed automatic attach is tried again later, and only a new reason
    /// for failing goes to the activity log. Asking again for a device that is attached or on its way is a no-op.
    /// </summary>
    private void StartAttach(string busId, bool automatic)
    {
        var key = KeyOf(busId);
        if (_attachments.ContainsKey(key) || !_attaching.TryAdd(key, true))
        {
            return;
        }

        Interlocked.Increment(ref _attachStarts);
        _lastTry[busId] = time.GetUtcNow();
        var epoch = EpochToken();
        var releasing = Releasing();
        _tasks.Run($"attach {busId}", async () =>
        {
            using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(_stopping, epoch);
            _inFlight[key] = cancellation;
            try
            {
                // A change that came first is still releasing what it ended; do not attach under its feet.
                await releasing.WaitAsync(cancellation.Token);
                await (UsesDriver ? AttachThroughDriverAsync(key, cancellation.Token) : AttachAsync(key, cancellation.Token));
            }
            catch (OperationCanceledException) when (cancellation.IsCancellationRequested)
            {
                // The connection changed or Koppel is stopping: the attach is not wanted any more.
            }
            catch (Exception ex)
            {
                await ReportFailureAsync(busId, ex, automatic);
            }
            finally
            {
                _inFlight.TryRemove(new KeyValuePair<AttachKey, CancellationTokenSource>(key, cancellation));
                _attaching.TryRemove(key, out _);
            }
        });
    }

    /// <summary>
    /// What goes wrong with a server, whichever path found out (an attach, the gateway, the link): a
    /// readable reason, in the log, and for a device in its fault and the activity log. Returns the reason.
    /// </summary>
    private async Task<string> ReportFailureAsync(string? busId, Exception ex, bool automatic)
    {
        var reason = RemoteFailure.Describe(ex);
        if (busId is null)
        {
            LogGatewayFailed("the device list", reason);
            return reason;
        }

        // The driver may have attached it by itself in the meantime, which is what was wanted.
        if (!_attachments.ContainsKey(KeyOf(busId)))
        {
            var repeated = automatic && _faults.GetValueOrDefault(busId) == reason;
            _faults[busId] = reason;
            LogAttachFailed(busId, reason);
            if (!repeated)
            {
                await activity.RecordAsync(Device(busId), ActivityReason.Attached, ActivityOutcome.Failed, $"{NameOf(busId)} was not attached", reason);
            }

            await RaiseAsync();
        }

        return reason;
    }

    /// <summary>Attaches the remembered devices that the server offers and nobody is using.</summary>
    private void AttachRemembered()
    {
        var remembered = _remembered.Current;
        if (_link != "connected" || remembered.ServerFingerprint is null || remembered.ServerFingerprint != connection.Current.ServerFingerprint)
        {
            return;
        }

        var now = time.GetUtcNow();
        foreach (var device in _remote.Devices.Where(d => d.State == "shared" && remembered.Expects(d)))
        {
            if (!_detaching.ContainsKey(device.BusId) && !_attachments.ContainsKey(KeyOf(device.BusId)) && (!_lastTry.TryGetValue(device.BusId, out var last) || now - last >= RetryInterval))
            {
                StartAttach(device.BusId, automatic: true);
            }
        }
    }

    /// <summary>Adds the device to <c>attached.json</c>, or takes it out.</summary>
    private Task<AttachedDevices> RememberAsync(string busId, bool attached) => _remembered.UpdateAsync(current =>
    {
        var fingerprint = connection.Current.ServerFingerprint;
        if (current.ServerFingerprint != fingerprint)
        {
            current = AttachedDevices.Empty with { ServerFingerprint = fingerprint };
        }

        var vidPid = _remote.Devices.FirstOrDefault(d => d.BusId == busId)?.VidPid;
        return current with
        {
            BusIds = attached ? [.. current.BusIds.Append(busId).Distinct()] : [.. current.BusIds.Where(b => b != busId)],
            VidPids = attached && vidPid is not null
                ? new Dictionary<string, string>(current.VidPids ?? new Dictionary<string, string>()) { [busId] = vidPid }
                : current.VidPids?.Where(p => p.Key != busId).ToDictionary(p => p.Key, p => p.Value),
        };
    }, CancellationToken.None);

    /// <summary>
    /// Serves one plain USB/IP connection from a USB/IP driver on this machine, such as usbip-win2 on the
    /// Windows host of Koppel's container. The request goes to the server through the same TLS session
    /// an attach uses. An import is tracked, so the UI shows the device attached here and can detach it.
    /// </summary>
    public async Task ServeGatewayAsync(Socket local, CancellationToken stoppingToken)
    {
        var from = (local.RemoteEndPoint as IPEndPoint)?.Address.ToString() ?? "this machine";
        await using var plain = new NetworkStream(local, ownsSocket: true);
        ushort replyCode = 0;
        string? busId = null;
        try
        {
            // A local peer that connects and says nothing must not hold the session open for ever.
            using var request = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
            request.CancelAfter(_handshakeTimeout);
            var header = await UsbIpProtocol.ReadHeaderAsync(plain, request.Token);
            replyCode = header.Code switch
            {
                UsbIpProtocol.OpReqDevList => UsbIpProtocol.OpRepDevList,
                UsbIpProtocol.OpReqImport => UsbIpProtocol.OpRepImport,
                _ => (ushort)0,
            };
            if (replyCode == 0)
            {
                return;
            }

            busId = header.Code == UsbIpProtocol.OpReqImport ? await UsbIpProtocol.ReadBusIdAsync(plain, request.Token) : null;
        }
        catch (OperationCanceledException) when (!stoppingToken.IsCancellationRequested)
        {
            LogGatewayFailed("the request", "the driver did not finish its request in time");
            return;
        }

        if (busId is not null && !UsbBusId.IsValid(busId))
        {
            LogGatewayFailed("an invalid bus ID", "not a USB bus ID");
            await plain.WriteAsync(UsbIpProtocol.Header(replyCode, UsbIpProtocol.StatusError), stoppingToken);
            return;
        }

        // With a driver on this machine the gateway serves only what Koppel itself asked the driver for: one
        // import of one device per request. Another local user's "usbip attach" and the driver's own attempts
        // to attach again after a drop are turned away; Koppel attaches again itself while it is wanted.
        PendingImport? booked = null;
        if (UsesDriver && (busId is null || !_pending.TryGetValue(KeyOf(busId), out booked) || !booked.TryClaim()))
        {
            LogGatewayFailed(busId ?? "the device list", "Koppel did not ask the driver for it");
            await plain.WriteAsync(UsbIpProtocol.Header(replyCode, UsbIpProtocol.StatusError), stoppingToken);
            return;
        }

        RemoteImport? import = null;
        try
        {
            if (settings.Current.Role == KoppelRole.Server)
            {
                throw new InvalidOperationException("This machine is a server, not a client.");
            }

            if (busId is null)
            {
                await UnderDeadlineAsync(async token =>
                {
                    await using (var session = await OpenSessionAsync(token))
                    {
                        await session.Stream.WriteAsync(UsbIpProtocol.DevListRequest(), token);
                        await session.Stream.CopyToAsync(plain, token);
                    }

                    return true;
                }, stoppingToken);
                return;
            }

            import = await ImportAsync(busId, stoppingToken);
            Check(UsbIpProtocol.ReadDevice(import.Record), busId, Offered(busId));
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (import is not null)
            {
                await import.DisposeAsync();
            }

            await ReportFailureAsync(busId, ex, automatic: false);
            await plain.WriteAsync(UsbIpProtocol.Header(replyCode, UsbIpProtocol.StatusError), stoppingToken);
            return;
        }

        await using (import)
        {
            var key = KeyOf(busId!);
            using var stop = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
            // An import this client's own Attach asked the driver for is "attached here", not a visitor.
            var mine = booked is not null;
            var attachment = new Attachment(null, stop, time.GetUtcNow(), mine ? null : from, mine);
            if (!_attachments.TryAdd(key, attachment))
            {
                LogGatewayFailed(busId!, "already attached");
                await plain.WriteAsync(UsbIpProtocol.Header(UsbIpProtocol.OpRepImport, UsbIpProtocol.StatusError), stoppingToken);
                return;
            }

            try
            {
                // Pass the server's device record on as it came, interfaces count and all.
                await plain.WriteAsync(UsbIpProtocol.Header(UsbIpProtocol.OpRepImport, UsbIpProtocol.StatusOk), stoppingToken);
                await plain.WriteAsync(import.Record, stoppingToken);

                _faults.TryRemove(busId!, out _);
                LogGatewayAttached(busId!, from);
                await activity.RecordAsync(Device(busId!), ActivityReason.Attached, ActivityOutcome.Succeeded, mine
                    ? $"{NameOf(busId!)} is attached here"
                    : $"{NameOf(busId!)} is attached through the gateway for {from}");
                await RaiseAsync();
                await UsbSockets.RelayAsync(import.Session, plain, stop.Token);
            }
            finally
            {
                if (_attachments.TryRemove(new KeyValuePair<AttachKey, Attachment>(key, attachment)))
                {
                    // Released on the other side, or the link dropped. A detach here records itself.
                    await activity.RecordAsync(Device(busId!), ActivityReason.Detached, ActivityOutcome.Succeeded, mine
                        ? $"{NameOf(busId!)} was detached when its connection ended"
                        : $"{NameOf(busId!)} was released by {from}");
                }

                await RaiseAsync();
            }
        }
    }

    /// <summary>Runs <paramref name="work"/> under the handshake deadline; running out of time is a <see cref="TimeoutException"/>, not a cancel.</summary>
    private async Task<T> UnderDeadlineAsync<T>(Func<CancellationToken, Task<T>> work, CancellationToken cancellationToken)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        deadline.CancelAfter(_handshakeTimeout);
        try
        {
            return await work(deadline.Token);
        }
        catch (OperationCanceledException) when (deadline.IsCancellationRequested && !cancellationToken.IsCancellationRequested)
        {
            throw new TimeoutException($"The server did not answer within {_handshakeTimeout.TotalSeconds:0} seconds.");
        }
    }

    /// <summary>
    /// Opens the USB data plane: a <c>usb</c> stream on the link, pinned to the server's certificate and
    /// carrying the link's token. USB/IP follows inside it. Both ends ping, so a server that vanishes
    /// without a goodbye ends the session in under a minute, not never. The caller sets the deadline
    /// through <paramref name="cancellationToken"/>.
    /// </summary>
    private async Task<RemoteStream> OpenSessionAsync(CancellationToken cancellationToken)
    {
        if (CurrentLink() is not { } remote || !_listed)
        {
            throw new InvalidOperationException("Not connected to a server.");
        }

        var opened = await remote.Link.OpenStreamAsync(UsbDataPlane.StreamType, null, cancellationToken);
        if (!opened.IsOpen)
        {
            throw new InvalidOperationException($"The server refused the USB connection: {opened.Refusal.Message}");
        }

        return new RemoteStream(opened.Stream);
    }

    /// <summary>Connects and imports <paramref name="busId"/>, all within one deadline. Returns the session and the server's device record.</summary>
    private Task<RemoteImport> ImportAsync(string busId, CancellationToken cancellationToken) => UnderDeadlineAsync(async token =>
    {
        var session = await OpenSessionAsync(token);
        try
        {
            await session.Stream.WriteAsync(UsbIpProtocol.ImportRequest(busId), token);
            var header = await UsbIpProtocol.ReadHeaderAsync(session.Stream, token);
            if (header.Code != UsbIpProtocol.OpRepImport || header.Status != UsbIpProtocol.StatusOk)
            {
                throw new InvalidOperationException("The server could not export the device. Someone else may be using it.");
            }

            var record = new byte[UsbIpProtocol.DeviceSize];
            await session.Stream.ReadExactlyAsync(record, token);
            return new RemoteImport(session, record);
        }
        catch
        {
            await session.DisposeAsync();
            throw;
        }
    }, cancellationToken);

    /// <summary>The device the server's list offers under this bus ID, as the user saw it when picking.</summary>
    private DeviceView? Offered(string busId) => _remote.Devices.FirstOrDefault(d => d.BusId == busId);

    /// <summary>
    /// The server answers an import with a device record. It must describe the device the user picked: a
    /// reply for another bus ID, or for another VID:PID than the list showed, is refused.
    /// </summary>
    internal static void Check(UsbDevice device, string busId, DeviceView? picked)
    {
        if (device.BusId != busId)
        {
            throw new InvalidOperationException($"The server answered with a different device than {busId}.");
        }

        if (picked is not null && device.VidPid != picked.VidPid)
        {
            throw new InvalidOperationException($"The device on {busId} is not the one that was picked ({picked.VidPid}); the server sent {device.VidPid}.");
        }
    }

    /// <summary>
    /// Asks the driver to attach from the gateway. The gateway sees the import and tracks the device;
    /// this adds the driver's port, which detaching needs.
    /// </summary>
    private async Task AttachThroughDriverAsync(AttachKey key, CancellationToken cancellationToken)
    {
        if (!connection.Current.IsComplete || !_listed)
        {
            throw new InvalidOperationException("Not connected to a server.");
        }

        if (_gatewayPort is not { } gatewayPort)
        {
            throw new InvalidOperationException(_gatewayFault ?? "The local USB/IP gateway is not running.");
        }

        var booking = new PendingImport();
        _pending[key] = booking;
        try
        {
            // The driver's own command has a time limit, and killing it half way could leave the attach
            // half done, so it is not cancelled; a change that came meanwhile is undone below.
            var driverPort = await driver.AttachAsync(IPAddress.Loopback.ToString(), gatewayPort, key.BusId, CancellationToken.None);
            _driverPorts[key] = new DriverAttachment(gatewayPort, driverPort);
            if (_attachments.TryGetValue(key, out var attachment) && attachment.Driver)
            {
                attachment.Port = driverPort;
                LogAttached(key.BusId, driverPort);
            }

            // Detached, or the connection changed, while the driver was at it: give back what it made. The
            // release takes the driver's entry out, and if it fails the entry stays so a later release tries again.
            if (cancellationToken.IsCancellationRequested)
            {
                _attachments.TryRemove(key, out var current);
                await ReleaseAsync(key, current ?? attachment, CancellationToken.None);
                await RaiseAsync();
                return;
            }

            await RaiseAsync();
        }
        finally
        {
            _pending.TryRemove(new KeyValuePair<AttachKey, PendingImport>(key, booking));
        }
    }

    private async Task AttachAsync(AttachKey key, CancellationToken cancellationToken)
    {
        if (attacher.Unavailable is { } reason)
        {
            throw new UsbHostException(reason);
        }

        var busId = key.BusId;
        var picked = Offered(busId) ?? throw new InvalidOperationException($"The server is not offering {busId}.");
        var import = await ImportAsync(busId, cancellationToken);
        try
        {
            var device = UsbIpProtocol.ReadDevice(import.Record);
            Check(device, busId, picked);
            cancellationToken.ThrowIfCancellationRequested();
            if (KeyOf(busId) != key)
            {
                throw new InvalidOperationException("The connection changed while attaching.");
            }

            var (kernelEnd, relayEnd) = await UsbSockets.CreateLoopbackPairAsync(cancellationToken);
            int port;
            try
            {
                port = attacher.Attach(kernelEnd, device.DevId, device.Speed);
            }
            catch
            {
                kernelEnd.Dispose();
                relayEnd.Dispose();
                throw;
            }

            var stop = new CancellationTokenSource();
            var attachment = new Attachment(port, stop, time.GetUtcNow());
            if (!_attachments.TryAdd(key, attachment))
            {
                // Something else attached it first; give the port back.
                try
                {
                    attacher.Detach(port);
                }
                catch (UsbHostException)
                {
                    // Already gone.
                }

                kernelEnd.Dispose();
                relayEnd.Dispose();
                stop.Dispose();
                throw new InvalidOperationException($"{busId} is already attached.");
            }

            _tasks.Run($"relay {busId}", async () =>
            {
                await using var relay = new NetworkStream(relayEnd, ownsSocket: true);
                try
                {
                    await UsbSockets.RelayAsync(import.Session, relay, stop.Token);
                }
                finally
                {
                    kernelEnd.Dispose();
                    await import.DisposeAsync();
                    stop.Dispose();
                    // Only this attach's own entry goes, and only then is the port freed: a detach and a
                    // new attach may have taken the key, and the port, in the meantime.
                    if (_attachments.TryRemove(new KeyValuePair<AttachKey, Attachment>(key, attachment)))
                    {
                        // The link dropped without a detach; free the port.
                        try
                        {
                            attacher.Detach(port);
                        }
                        catch (UsbHostException)
                        {
                            // Already gone.
                        }
                    }

                    await RaiseAsync();
                }
            });

            _faults.TryRemove(busId, out _);
            LogAttached(busId, port);
            await activity.RecordAsync(Device(busId), ActivityReason.Attached, ActivityOutcome.Succeeded, $"{NameOf(busId)} is attached here on virtual port {port}");
            await RaiseAsync();

            if (cancellationToken.IsCancellationRequested && _attachments.TryRemove(new KeyValuePair<AttachKey, Attachment>(key, attachment)))
            {
                // The connection changed while this attach finished.
                await ReleaseAsync(key, attachment, CancellationToken.None);
            }
        }
        catch
        {
            await import.DisposeAsync();
            throw;
        }
    }

    private async Task SetLinkAsync(string link, string? message)
    {
        var previous = _link;
        _link = link;
        _linkMessage = message;
        if (previous != link && (link == "connected" || (previous == "connected" && link is "error" or "connecting")))
        {
            var server = connection.Current.ServerUrl is { } url ? new Uri(url).Host : "the server";
            var entity = new EntityRef("link", server, "link");
            await (link == "connected"
                ? activity.RecordAsync(entity, ActivityReason.LinkUp, ActivityOutcome.Succeeded, $"Linked to {server}")
                : activity.RecordAsync(entity, ActivityReason.LinkDown, ActivityOutcome.Failed, $"Lost the link to {server}", message));
        }

        await RaiseAsync();
    }

    private AttachKey KeyOf(string busId) => new(connection.Current.ClientId ?? "", busId);

    private EntityRef Device(string busId) => new(busId, NameOf(busId), "device");

    private string NameOf(string busId) => _remote.Devices.FirstOrDefault(d => d.BusId == busId)?.Name ?? busId;

    private Task RaiseAsync() => Changed.RaiseAsync(Current, logger);

    /// <summary>The link to one saved server, and the connection it was made for, which moves on as the link learns.</summary>
    internal sealed class RemoteLink(ClientConnection connection, SkakelLink link)
    {
        public ClientConnection Connection
        {
            get => Volatile.Read(ref field);
            set => Volatile.Write(ref field, value);
        } = connection;

        public SkakelLink Link { get; } = link;
    }

    /// <summary>A <c>usb</c> stream as a byte stream. Disposing it closes the stream normally.</summary>
    private sealed class RemoteStream(SkakelStreamConnection connection) : IAsyncDisposable
    {
        public Stream Stream { get; } = WebSocketStream.Create(connection.WebSocket, WebSocketMessageType.Binary, ownsWebSocket: false);

        public async ValueTask DisposeAsync()
        {
            await Stream.DisposeAsync();
            await connection.DisposeAsync();
        }
    }

    /// <summary>The stream to the server for one imported device, and the device record it sent.</summary>
    private sealed class RemoteImport(RemoteStream remote, byte[] record) : IAsyncDisposable
    {
        public Stream Session => remote.Stream;

        public byte[] Record { get; } = record;

        public ValueTask DisposeAsync() => remote.DisposeAsync();
    }

    /// <param name="Port">The virtual port, or the driver's port for a driver attach; null for another gateway attach.</param>
    /// <param name="Via">Where another gateway attach came from.</param>
    /// <param name="Driver">This client's own attach, made through the driver and the gateway.</param>
    /// <remarks>A class, so removing "this attach's own entry" means this very object.</remarks>
    private sealed class Attachment(int? port, CancellationTokenSource stop, DateTimeOffset since, string? via = null, bool driver = false)
    {
        public int? Port { get; set; } = port;

        public CancellationTokenSource Stop { get; } = stop;

        public DateTimeOffset Since { get; } = since;

        public string? Via { get; } = via;

        public bool Driver { get; } = driver;
    }

    /// <summary>Koppel's request to the driver to import one device, which the gateway accepts exactly once.</summary>
    private sealed class PendingImport
    {
        private int _claimed;

        public bool TryClaim() => Interlocked.CompareExchange(ref _claimed, 1, 0) == 0;
    }

    /// <summary>What the driver holds for one device: the gateway port it was pointed at, and its own port number.</summary>
    private sealed record DriverAttachment(int GatewayPort, int DriverPort);

    [LoggerMessage(Level = LogLevel.Information, Message = "Attached {BusId} on virtual port {Port}")]
    private partial void LogAttached(string busId, int port);

    [LoggerMessage(Level = LogLevel.Information, Message = "Attached {BusId} through the gateway for {From}")]
    private partial void LogGatewayAttached(string busId, string from);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Gateway request for {Subject} failed: {Reason}")]
    private partial void LogGatewayFailed(string subject, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Attaching {BusId} failed: {Reason}")]
    private partial void LogAttachFailed(string busId, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Detaching {BusId} failed: {Reason}")]
    private partial void LogDetachFailed(string busId, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "The link to the server failed: {Failure}")]
    private partial void LogLinkFailed(string failure);

    [LoggerMessage(Level = LogLevel.Information, Message = "The server moved from {OldAddress} to {NewAddress}; the saved connection follows it")]
    private partial void LogMoved(string oldAddress, string newAddress);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Ignored {Count} devices from the server with a bus ID that is not valid")]
    private partial void LogInvalidDevices(int count);

    [LoggerMessage(Level = LogLevel.Error, Message = "Could not apply a change of connection or role")]
    private partial void LogChangeFailed(Exception exception);
}
