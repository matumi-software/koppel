using System.Collections.Concurrent;
using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using System.Text;
using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Core.Auth;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Koppel.Core.Usb;
using Koppel.Settings;
using Microsoft.AspNetCore.SignalR.Client;

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
/// <param name="ServerFingerprint">The server the bus IDs belong to. Another server starts a new list.</param>
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
/// server's fingerprint as well: the same bus ID on another server is another device.
/// </summary>
internal readonly record struct AttachKey(string Server, string BusId);

/// <summary>
/// Client role: keeps a live SignalR link to the configured server for its device list, and attaches
/// and detaches devices. An attach opens a TLS connection to the server's USB port, authenticates with
/// the bearer token, imports the device over USB/IP, then hands one end of a loopback pair to the
/// kernel's virtual host controller and relays the other end through TLS.
/// </summary>
internal sealed partial class UsbClient(
    KoppelPaths paths,
    ConnectionStore connection,
    SettingsStore settings,
    IUsbAttacher attacher,
    IUsbIpDriver driver,
    TimeProvider time,
    IActivityRecorder activity,
    IConfiguration configuration,
    ILogger<UsbClient> logger) : BackgroundService
{
    /// <summary>How often a remembered device that is not attached yet is tried again.</summary>
    private static readonly TimeSpan RetryInterval = TimeSpan.FromSeconds(5);

    private static readonly TimeSpan MinBackoff = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan MaxBackoff = TimeSpan.FromSeconds(60);

    /// <summary>
    /// One deadline for everything before USB traffic flows: connecting, TLS, the token, the preface, the
    /// import reply and the device record. A server that accepts and then says nothing ends here.
    /// </summary>
    private readonly TimeSpan _handshakeTimeout = TimeSpan.FromSeconds(configuration.GetValue("Usb:AttachTimeoutSeconds", 20));

    private readonly JsonConfigFile<AttachedDevices> _remembered = new(
        Path.Combine(paths.ConfigDirectory, "attached.json"), AttachedDevices.Empty, KoppelJson.Files, logger);

    private readonly BackgroundTasks _tasks = new(logger);
    private readonly ConcurrentDictionary<AttachKey, Attachment> _attachments = new();
    private readonly ConcurrentDictionary<AttachKey, bool> _attaching = new();
    private readonly ConcurrentDictionary<AttachKey, bool> _driverAttaching = new();
    private readonly ConcurrentDictionary<AttachKey, DriverAttachment> _driverPorts = new();
    private readonly ConcurrentDictionary<string, DateTimeOffset> _lastTry = new();
    private readonly ConcurrentDictionary<string, string> _faults = new();
    private readonly object _gate = new();
    private RemoteDevices _remote = new([], 0);
    private string _link = "unconfigured";
    private string? _linkMessage;
    private CancellationTokenSource? _session;
    private CancellationTokenSource _epoch = new();
    private ServerLink? _serverLink;
    private (bool Client, ClientConnection Connection) _applied;
    private CancellationToken _stopping;
    private int? _gatewayPort;
    private string? _gatewayFault;

    public event Func<ClientDevices, Task>? Changed;

    public ClientDevices Current
    {
        get
        {
            var fingerprint = connection.Current.ServerFingerprint ?? "";
            return new(
                _link,
                _linkMessage,
                [.. _remote.Devices.Select(d =>
                {
                    var a = _attachments.GetValueOrDefault(new AttachKey(fingerprint, d.BusId));
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
        lock (_gate)
        {
            _serverLink?.Dispose();
            _serverLink = null;
        }

        base.Dispose();
    }

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

    public async Task DetachAsync(string busId)
    {
        await RememberAsync(busId, attached: false);
        _faults.TryRemove(busId, out _);
        var key = KeyOf(busId);
        _attachments.TryRemove(key, out var attachment);
        if (attachment is not null || _driverPorts.ContainsKey(key))
        {
            await ReleaseAsync(key, attachment, CancellationToken.None);
        }

        await RaiseAsync();
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
            try
            {
                await attachment.Stop.CancelAsync();
            }
            catch (ObjectDisposedException)
            {
                // The session ended by itself first, as it does once a driver lets go of the device.
            }
        }
    }

    /// <summary>Stops the driver's attempts to attach the device and detaches its port. False when the driver holds none.</summary>
    private async Task<bool> ReleaseDriverAsync(AttachKey key, CancellationToken cancellationToken)
    {
        if (!_driverPorts.TryGetValue(key, out var held))
        {
            return false;
        }

        await driver.DetachAsync(IPAddress.Loopback.ToString(), held.GatewayPort, key.BusId, held.DriverPort, cancellationToken);
        _driverPorts.TryRemove(key, out _);
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

    /// <summary>
    /// A new server, new credentials, or a role that is no longer Client or Both ends every attachment
    /// first, so nothing stays attached to a server the user moved away from, and then restarts the link.
    /// A change that touches neither (the listen port, say) leaves the link alone.
    /// </summary>
    private async Task OnChangedAsync()
    {
        var now = Settled();
        lock (_gate)
        {
            if (_applied == now)
            {
                return;
            }

            _applied = now;
        }

        try
        {
            await EndEpochAsync();
            await ReleaseAllAsync();
            _faults.Clear();
            _lastTry.Clear();
            _remote = new([], 0);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            LogChangeFailed(ex);
        }

        CancelSession();
        await RaiseAsync();
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

    /// <summary>The link's token is cancelled under the lock that also ends its life, so it is never cancelled after it was disposed.</summary>
    private void CancelSession()
    {
        lock (_gate)
        {
            try
            {
                _session?.Cancel();
            }
            catch (ObjectDisposedException)
            {
                // The link ended on its own just now.
            }
        }
    }

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
        var backoff = MinBackoff;
        try
        {
            while (!stoppingToken.IsCancellationRequested)
            {
                using var session = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
                lock (_gate)
                {
                    _session = session;
                }

                try
                {
                    await RunLinkAsync(session.Token);
                    backoff = MinBackoff;
                }
                catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
                {
                    break;
                }
                catch (OperationCanceledException)
                {
                    // Settings or connection changed; start over with the new values.
                    backoff = MinBackoff;
                }
                catch (Exception ex)
                {
                    // Whatever went wrong, the host keeps running and the UI says what happened.
                    var reason = RemoteFailure.Describe(ex);
                    LogLinkFailed(reason, ex);
                    try
                    {
                        await SetLinkAsync("error", reason);
                        await Task.Delay(backoff, session.Token);
                    }
                    catch (OperationCanceledException)
                    {
                        // Stopping, or the settings changed and the next try may differ.
                    }
                    catch (Exception inner)
                    {
                        LogLinkFailed(inner.Message, inner);
                    }

                    backoff = TimeSpan.FromTicks(Math.Min(backoff.Ticks * 2, MaxBackoff.Ticks));
                }
                finally
                {
                    lock (_gate)
                    {
                        _session = null;
                    }
                }
            }
        }
        finally
        {
            connection.Changed -= Restart;
            settings.Changed -= Restart;
        }
    }

    private async Task RunLinkAsync(CancellationToken cancellationToken)
    {
        var c = connection.Current;
        if (settings.Current.Role == KoppelRole.Server || !c.IsComplete)
        {
            _remote = new([], 0);
            await SetLinkAsync("unconfigured", null);
            await Task.Delay(Timeout.Infinite, cancellationToken);
            return;
        }

        await SetLinkAsync("connecting", null);
        // The same token client serves the hub, every attach and every gateway session, so a token is
        // fetched once and reused until shortly before it expires.
        var serverLink = ServerLinkFor();
        var server = serverLink.Server;
        await using var hub = new HubConnectionBuilder()
            .WithUrl(new Uri(server, "/api/hubs/remote"), options =>
            {
                options.AccessTokenProvider = async () => await serverLink.Tokens.GetAccessTokenAsync(server, c.ClientId!, c.ClientSecret!, cancellationToken);
                options.HttpMessageHandlerFactory = _ => ServerCertificate.CreatePinnedHandler(c.ServerFingerprint!);
                options.WebSocketConfiguration = ws => ws.RemoteCertificateValidationCallback =
                    (_, certificate, _, _) => ServerCertificate.Matches(certificate as System.Security.Cryptography.X509Certificates.X509Certificate2, c.ServerFingerprint);
            })
            .WithAutomaticReconnect()
            .AddJsonProtocol(options => KoppelJson.Configure(options.PayloadSerializerOptions))
            .Build();

        var closed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<RemoteDevices>("DevicesChanged", async devices =>
        {
            _remote = Sanitize(devices);
            await RaiseAsync();
            AttachRemembered();
        });
        hub.Reconnecting += _ => SetLinkAsync("connecting", null);
        hub.Reconnected += _ => SetLinkAsync("connected", null);
        hub.Closed += ex =>
        {
            closed.TrySetResult();
            return SetLinkAsync("error", ex is null ? "The server closed the link." : RemoteFailure.Describe(ex));
        };

        await hub.StartAsync(cancellationToken);
        await SetLinkAsync("connected", null);

        // A remembered device that could not be attached yet, because it was in use or the server was
        // still starting, is tried again until the link closes.
        using var retry = new PeriodicTimer(RetryInterval, time);
        while (!closed.Task.IsCompleted)
        {
            AttachRemembered();
            await Task.WhenAny(closed.Task, retry.WaitForNextTickAsync(cancellationToken).AsTask());
            cancellationToken.ThrowIfCancellationRequested();
        }

        await Task.Delay(TimeSpan.FromSeconds(5), cancellationToken);
    }

    /// <summary>
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

        return new RemoteDevices(valid, received?.UsbPort is > 0 and <= 65535 ? received.UsbPort : 0);
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

        _lastTry[busId] = time.GetUtcNow();
        var epoch = EpochToken();
        _tasks.Run($"attach {busId}", async () =>
        {
            using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(_stopping, epoch);
            try
            {
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
            if (!_attachments.ContainsKey(KeyOf(device.BusId)) && (!_lastTry.TryGetValue(device.BusId, out var last) || now - last >= RetryInterval))
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
                    var (tcp, tls) = await OpenSessionAsync(token);
                    using (tcp)
                    await using (tls)
                    {
                        await tls.WriteAsync(UsbIpProtocol.DevListRequest(), token);
                        await tls.CopyToAsync(plain, token);
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
            var mine = _driverAttaching.ContainsKey(key);
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
                await UsbSockets.RelayAsync(import.Tls, plain, stop.Token);
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

    /// <summary>
    /// The one token client for the saved connection, with its pinned HTTP client. A new one is built
    /// when the connection changes, so a token for another server or secret is never reused.
    /// </summary>
    internal ServerLink ServerLinkFor()
    {
        lock (_gate)
        {
            var c = connection.Current;
            if (!c.IsComplete)
            {
                throw new InvalidOperationException("Not connected to a server.");
            }

            if (_serverLink is null || _serverLink.Connection != c)
            {
                _serverLink?.Dispose();
                _serverLink = new ServerLink(c, time);
            }

            return _serverLink;
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
    /// Opens a TLS connection to the server's USB port and proves who this client is. USB/IP follows.
    /// The caller sets the deadline through <paramref name="cancellationToken"/>.
    /// </summary>
    private async Task<(TcpClient Tcp, SslStream Tls)> OpenSessionAsync(CancellationToken cancellationToken)
    {
        var link = ServerLinkFor();
        if (_remote.UsbPort == 0)
        {
            throw new InvalidOperationException("Not connected to a server.");
        }

        var c = link.Connection;
        var token = await link.Tokens.GetAccessTokenAsync(link.Server, c.ClientId!, c.ClientSecret!, cancellationToken);

        var tcp = new TcpClient { NoDelay = true };
        SslStream? tls = null;
        try
        {
            await tcp.ConnectAsync(link.Server.Host, _remote.UsbPort, cancellationToken);
            // A server that vanishes without a goodbye ends the session in about a minute, not never.
            UsbSockets.EnableKeepAlive(tcp.Client);
            tls = new SslStream(tcp.GetStream(), leaveInnerStreamOpen: false);
            await tls.AuthenticateAsClientAsync(new SslClientAuthenticationOptions
            {
                TargetHost = link.Server.Host,
                RemoteCertificateValidationCallback = (_, certificate, _, _) =>
                    ServerCertificate.Matches(certificate as System.Security.Cryptography.X509Certificates.X509Certificate2, c.ServerFingerprint),
            }, cancellationToken);
            await tls.WriteAsync(Encoding.ASCII.GetBytes($"{UsbIpTlsServer.Preface}{token}\n"), cancellationToken);
            var answer = await UsbIpTlsServer.ReadLineAsync(tls, cancellationToken);
            if (answer != "OK")
            {
                throw new InvalidOperationException($"The server refused the connection: {answer ?? "no answer"}.");
            }

            return (tcp, tls);
        }
        catch
        {
            if (tls is not null)
            {
                await tls.DisposeAsync();
            }

            tcp.Dispose();
            throw;
        }
    }

    /// <summary>Connects and imports <paramref name="busId"/>, all within one deadline. Returns the session and the server's device record.</summary>
    private Task<RemoteImport> ImportAsync(string busId, CancellationToken cancellationToken) => UnderDeadlineAsync(async token =>
    {
        var (tcp, tls) = await OpenSessionAsync(token);
        try
        {
            await tls.WriteAsync(UsbIpProtocol.ImportRequest(busId), token);
            var header = await UsbIpProtocol.ReadHeaderAsync(tls, token);
            if (header.Code != UsbIpProtocol.OpRepImport || header.Status != UsbIpProtocol.StatusOk)
            {
                throw new InvalidOperationException("The server could not export the device. Someone else may be using it.");
            }

            var record = new byte[UsbIpProtocol.DeviceSize];
            await tls.ReadExactlyAsync(record, token);
            return new RemoteImport(tcp, tls, record);
        }
        catch
        {
            await tls.DisposeAsync();
            tcp.Dispose();
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
        if (!connection.Current.IsComplete || _remote.UsbPort == 0)
        {
            throw new InvalidOperationException("Not connected to a server.");
        }

        if (_gatewayPort is not { } gatewayPort)
        {
            throw new InvalidOperationException(_gatewayFault ?? "The local USB/IP gateway is not running.");
        }

        _driverAttaching[key] = true;
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

            if (cancellationToken.IsCancellationRequested)
            {
                _attachments.TryRemove(key, out _);
                await ReleaseAsync(key, attachment, CancellationToken.None);
                return;
            }

            await RaiseAsync();
        }
        finally
        {
            _driverAttaching.TryRemove(key, out _);
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
                    await UsbSockets.RelayAsync(import.Tls, relay, stop.Token);
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

    private AttachKey KeyOf(string busId) => new(connection.Current.ServerFingerprint ?? "", busId);

    private EntityRef Device(string busId) => new(busId, NameOf(busId), "device");

    private string NameOf(string busId) => _remote.Devices.FirstOrDefault(d => d.BusId == busId)?.Name ?? busId;

    private Task RaiseAsync() => Changed.RaiseAsync(Current, logger);

    /// <summary>The TLS session to the server for one imported device, and the device record it sent.</summary>
    private sealed class RemoteImport(TcpClient tcp, SslStream tls, byte[] record) : IAsyncDisposable
    {
        public SslStream Tls { get; } = tls;

        public byte[] Record { get; } = record;

        public async ValueTask DisposeAsync()
        {
            await Tls.DisposeAsync();
            tcp.Dispose();
        }
    }

    /// <summary>
    /// The saved connection with the one token client and pinned HTTP client made for it. Everything that
    /// talks to the server over HTTPS shares this, so the cached token is reused.
    /// </summary>
    internal sealed class ServerLink : IDisposable
    {
        private readonly HttpClient _http;

        public ServerLink(ClientConnection connection, TimeProvider time)
        {
            Connection = connection;
            Server = new Uri(connection.ServerUrl!);
            _http = new HttpClient(ServerCertificate.CreatePinnedHandler(connection.ServerFingerprint!));
            Tokens = new KoppelTokenClient(_http, time);
        }

        public ClientConnection Connection { get; }

        public Uri Server { get; }

        public KoppelTokenClient Tokens { get; }

        public void Dispose()
        {
            Tokens.Dispose();
            _http.Dispose();
        }
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

    [LoggerMessage(Level = LogLevel.Warning, Message = "The link to the server failed: {Reason}")]
    private partial void LogLinkFailed(string reason, Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Ignored {Count} devices from the server with a bus ID that is not valid")]
    private partial void LogInvalidDevices(int count);

    [LoggerMessage(Level = LogLevel.Error, Message = "Could not apply a change of connection or role")]
    private partial void LogChangeFailed(Exception exception);
}
