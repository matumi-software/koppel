using System.Collections.Concurrent;
using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using System.Text;
using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Core.Auth;
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
public sealed record ClientDevices(string Link, string? LinkMessage, IReadOnlyList<ClientDeviceView> Devices, string? Unavailable, bool Gateway = false);

/// <summary>
/// Client role: keeps a live SignalR link to the configured server for its device list, and attaches
/// and detaches devices. An attach opens a TLS connection to the server's USB port, authenticates with
/// the bearer token, imports the device over USB/IP, then hands one end of a loopback pair to the
/// kernel's virtual host controller and relays the other end through TLS.
/// </summary>
internal sealed partial class UsbClient(
    ConnectionStore connection,
    SettingsStore settings,
    IUsbAttacher attacher,
    IUsbIpDriver driver,
    TimeProvider time,
    IActivityRecorder activity,
    ILogger<UsbClient> logger) : BackgroundService
{
    private readonly ConcurrentDictionary<string, Attachment> _attachments = new();
    private readonly ConcurrentDictionary<string, string> _faults = new();
    private readonly ConcurrentDictionary<string, bool> _driverAttaching = new();
    private RemoteDevices _remote = new([], 0);
    private string _link = "unconfigured";
    private string? _linkMessage;
    private CancellationTokenSource? _session;
    private int? _gatewayPort;

    public event Func<ClientDevices, Task>? Changed;

    public ClientDevices Current => new(
        _link,
        _linkMessage,
        [.. _remote.Devices.Select(d =>
        {
            var a = _attachments.GetValueOrDefault(d.BusId);
            return new ClientDeviceView(d, a is not null, a?.Port, a?.Since, _faults.GetValueOrDefault(d.BusId), a?.Via);
        })],
        UsesDriver ? null : attacher.Unavailable,
        _gatewayPort is not null && !UsesDriver);

    /// <summary>
    /// True when attaching goes through a driver on this machine (usbip-win2 on Windows) by way of the
    /// gateway, rather than through this kernel's virtual host controller.
    /// </summary>
    private bool UsesDriver => _gatewayPort is not null && driver.Unavailable is null;

    /// <summary>Called by <see cref="UsbIpGateway"/> with its port when it starts listening, and null when it stops.</summary>
    public Task SetGatewayAsync(int? port)
    {
        _gatewayPort = port;
        return RaiseAsync();
    }

    /// <summary>Starts attaching in the background. The outcome reaches the UI through <see cref="Changed"/>.</summary>
    public void BeginAttach(string busId)
    {
        _faults.TryRemove(busId, out _);
        _ = Task.Run(async () =>
        {
            try
            {
                await (UsesDriver ? AttachThroughDriverAsync(busId) : AttachAsync(busId));
            }
            catch (Exception ex) when (ex is UsbHostException or IOException or SocketException or TokenRequestException or System.Security.Authentication.AuthenticationException or OperationCanceledException or InvalidOperationException)
            {
                _faults[busId] = ex.Message;
                LogAttachFailed(busId, ex.Message);
                await activity.RecordAsync(Device(busId), ActivityReason.Attached, ActivityOutcome.Failed, $"{NameOf(busId)} was not attached", ex.Message);
                await RaiseAsync();
            }
        });
    }

    public async Task DetachAsync(string busId)
    {
        _faults.TryRemove(busId, out _);
        if (_attachments.TryRemove(busId, out var attachment))
        {
            try
            {
                // A driver attach is undone by the driver, which would otherwise attach it again.
                // Another gateway attach has no port here; stopping its relay unplugs it on the other side.
                if (attachment.Driver)
                {
                    if (attachment.Port is { } driverPort && _gatewayPort is { } gatewayPort)
                    {
                        await driver.DetachAsync(IPAddress.Loopback.ToString(), gatewayPort, busId, driverPort, CancellationToken.None);
                    }
                }
                else if (attachment.Via is null && attachment.Port is { } port)
                {
                    attacher.Detach(port);
                }

                await activity.RecordAsync(Device(busId), ActivityReason.Detached, ActivityOutcome.Succeeded, $"{NameOf(busId)} was detached");
            }
            catch (UsbHostException ex)
            {
                _faults[busId] = ex.Message;
            }

            try
            {
                await attachment.Stop.CancelAsync();
            }
            catch (ObjectDisposedException)
            {
                // The session ended by itself first, as it does once a driver lets go of the device.
            }
        }

        await RaiseAsync();
    }

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        Task Restart<T>(T _)
        {
            _session?.Cancel();
            return Task.CompletedTask;
        }

        connection.Changed += Restart;
        settings.Changed += Restart;
        try
        {
            while (!stoppingToken.IsCancellationRequested)
            {
                using var session = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
                _session = session;
                try
                {
                    await RunLinkAsync(session.Token);
                }
                catch (OperationCanceledException) when (!stoppingToken.IsCancellationRequested)
                {
                    // Settings or connection changed; start over with the new values.
                }
                catch (Exception ex) when (ex is HttpRequestException or TokenRequestException or IOException or InvalidOperationException)
                {
                    await SetLinkAsync("error", ex.Message);
                    await Task.Delay(TimeSpan.FromSeconds(10), stoppingToken);
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
        var server = new Uri(c.ServerUrl!);
        using var tokenHttp = new HttpClient(ServerCertificate.CreatePinnedHandler(c.ServerFingerprint!));
        using var tokens = new KoppelTokenClient(tokenHttp, time);
        await using var hub = new HubConnectionBuilder()
            .WithUrl(new Uri(server, "/api/hubs/remote"), options =>
            {
                options.AccessTokenProvider = async () => await tokens.GetAccessTokenAsync(server, c.ClientId!, c.ClientSecret!, cancellationToken);
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
            _remote = devices;
            await RaiseAsync();
        });
        hub.Reconnecting += _ => SetLinkAsync("connecting", null);
        hub.Reconnected += _ => SetLinkAsync("connected", null);
        hub.Closed += ex =>
        {
            closed.TrySetResult();
            return SetLinkAsync("error", ex?.Message ?? "The server closed the link.");
        };

        await hub.StartAsync(cancellationToken);
        await SetLinkAsync("connected", null);
        await closed.Task.WaitAsync(cancellationToken);
        await Task.Delay(TimeSpan.FromSeconds(5), cancellationToken);
    }

    /// <summary>
    /// Serves one plain USB/IP connection from a USB/IP driver on this machine, such as usbip-win2 on the
    /// Windows host of Koppel's container. The request goes to the server through the same TLS session
    /// an attach uses. An import is tracked, so the UI shows the device attached here and can detach it.
    /// </summary>
    public async Task ServeGatewayAsync(Socket local, CancellationToken stoppingToken)
    {
        var from = (local.RemoteEndPoint as IPEndPoint)?.Address.ToString() ?? "this machine";
        await using var plain = new NetworkStream(local, ownsSocket: true);
        var request = await UsbIpProtocol.ReadHeaderAsync(plain, stoppingToken);
        var replyCode = request.Code switch
        {
            UsbIpProtocol.OpReqDevList => UsbIpProtocol.OpRepDevList,
            UsbIpProtocol.OpReqImport => UsbIpProtocol.OpRepImport,
            _ => (ushort)0,
        };
        if (replyCode == 0)
        {
            return;
        }

        var busId = request.Code == UsbIpProtocol.OpReqImport ? await UsbIpProtocol.ReadBusIdAsync(plain, stoppingToken) : null;
        TcpClient tcp;
        SslStream tls;
        try
        {
            if (settings.Current.Role == KoppelRole.Server)
            {
                throw new InvalidOperationException("This machine is a server, not a client.");
            }

            (tcp, tls) = await OpenSessionAsync(stoppingToken);
        }
        catch (Exception ex) when (ex is IOException or SocketException or TokenRequestException or System.Security.Authentication.AuthenticationException or InvalidOperationException or HttpRequestException)
        {
            LogGatewayFailed(busId ?? "the device list", ex.Message);
            await plain.WriteAsync(UsbIpProtocol.Header(replyCode, UsbIpProtocol.StatusError), stoppingToken);
            return;
        }

        using (tcp)
        await using (tls)
        {
            if (busId is null)
            {
                await tls.WriteAsync(UsbIpProtocol.DevListRequest(), stoppingToken);
                await tls.CopyToAsync(plain, stoppingToken);
                return;
            }

            await tls.WriteAsync(UsbIpProtocol.ImportRequest(busId), stoppingToken);
            var reply = await UsbIpProtocol.ReadHeaderAsync(tls, stoppingToken);
            if (reply.Code != UsbIpProtocol.OpRepImport || reply.Status != UsbIpProtocol.StatusOk)
            {
                await plain.WriteAsync(UsbIpProtocol.Header(UsbIpProtocol.OpRepImport, UsbIpProtocol.StatusError), stoppingToken);
                await activity.RecordAsync(Device(busId), ActivityReason.Attached, ActivityOutcome.Failed, $"{NameOf(busId)} was not attached through the gateway", "The server could not export the device. Someone else may be using it.");
                return;
            }

            // Pass the server's device record on as it came, interfaces count and all.
            var device = new byte[UsbIpProtocol.DeviceSize];
            await tls.ReadExactlyAsync(device, stoppingToken);
            await plain.WriteAsync(UsbIpProtocol.Header(UsbIpProtocol.OpRepImport, UsbIpProtocol.StatusOk), stoppingToken);
            await plain.WriteAsync(device, stoppingToken);

            using var stop = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
            _faults.TryRemove(busId, out _);
            // An import this client's own Attach asked the driver for is "attached here", not a visitor.
            var mine = _driverAttaching.ContainsKey(busId);
            _attachments[busId] = new Attachment(null, stop, time.GetUtcNow(), mine ? null : from, Driver: mine);
            LogGatewayAttached(busId, from);
            await activity.RecordAsync(Device(busId), ActivityReason.Attached, ActivityOutcome.Succeeded, mine
                ? $"{NameOf(busId)} is attached here"
                : $"{NameOf(busId)} is attached through the gateway for {from}");
            await RaiseAsync();
            try
            {
                await UsbSockets.RelayAsync(tls, plain, stop.Token);
            }
            finally
            {
                if (_attachments.TryRemove(busId, out _))
                {
                    // Released on the other side, or the link dropped. A detach here records itself.
                    await activity.RecordAsync(Device(busId), ActivityReason.Detached, ActivityOutcome.Succeeded, mine
                        ? $"{NameOf(busId)} was detached when its connection ended"
                        : $"{NameOf(busId)} was released by {from}");
                }

                await RaiseAsync();
            }
        }
    }

    /// <summary>Opens a TLS connection to the server's USB port and proves who this client is. USB/IP follows.</summary>
    private async Task<(TcpClient Tcp, SslStream Tls)> OpenSessionAsync(CancellationToken cancellationToken)
    {
        var c = connection.Current;
        if (!c.IsComplete || _remote.UsbPort == 0)
        {
            throw new InvalidOperationException("Not connected to a server.");
        }

        var server = new Uri(c.ServerUrl!);
        using var tokenHttp = new HttpClient(ServerCertificate.CreatePinnedHandler(c.ServerFingerprint!));
        using var tokens = new KoppelTokenClient(tokenHttp, time);
        var token = await tokens.GetAccessTokenAsync(server, c.ClientId!, c.ClientSecret!, cancellationToken);

        var tcp = new TcpClient { NoDelay = true };
        SslStream? tls = null;
        try
        {
            await tcp.ConnectAsync(server.Host, _remote.UsbPort, cancellationToken);
            tls = new SslStream(tcp.GetStream(), leaveInnerStreamOpen: false,
                (_, certificate, _, _) => ServerCertificate.Matches(certificate as System.Security.Cryptography.X509Certificates.X509Certificate2, c.ServerFingerprint));
            await tls.AuthenticateAsClientAsync(server.Host);
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

    /// <summary>
    /// Asks the driver to attach from the gateway. The gateway sees the import and tracks the device;
    /// this adds the driver's port, which detaching needs.
    /// </summary>
    private async Task AttachThroughDriverAsync(string busId)
    {
        if (!connection.Current.IsComplete || _remote.UsbPort == 0 || _gatewayPort is not { } gatewayPort)
        {
            throw new InvalidOperationException("Not connected to a server.");
        }

        _driverAttaching[busId] = true;
        try
        {
            var driverPort = await driver.AttachAsync(IPAddress.Loopback.ToString(), gatewayPort, busId, CancellationToken.None);
            if (_attachments.TryGetValue(busId, out var attachment) && attachment.Driver)
            {
                _attachments[busId] = attachment with { Port = driverPort };
                LogAttached(busId, driverPort);
            }

            await RaiseAsync();
        }
        finally
        {
            _driverAttaching.TryRemove(busId, out _);
        }
    }

    private async Task AttachAsync(string busId)
    {
        if (attacher.Unavailable is { } reason)
        {
            throw new UsbHostException(reason);
        }

        var (tcp, tls) = await OpenSessionAsync(CancellationToken.None);
        try
        {
            await tls.WriteAsync(UsbIpProtocol.ImportRequest(busId));
            var header = await UsbIpProtocol.ReadHeaderAsync(tls, CancellationToken.None);
            if (header.Code != UsbIpProtocol.OpRepImport || header.Status != UsbIpProtocol.StatusOk)
            {
                throw new InvalidOperationException("The server could not export the device. Someone else may be using it.");
            }

            var device = await UsbIpProtocol.ReadImportedDeviceAsync(tls, CancellationToken.None);
            var (kernelEnd, relayEnd) = await UsbSockets.CreateLoopbackPairAsync(CancellationToken.None);
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
            _attachments[busId] = new Attachment(port, stop, time.GetUtcNow());
            LogAttached(busId, port);
            await activity.RecordAsync(Device(busId), ActivityReason.Attached, ActivityOutcome.Succeeded, $"{NameOf(busId)} is attached here on virtual port {port}");
            await RaiseAsync();

            _ = Task.Run(async () =>
            {
                await using var relay = new NetworkStream(relayEnd, ownsSocket: true);
                try
                {
                    await UsbSockets.RelayAsync(tls, relay, stop.Token);
                }
                finally
                {
                    kernelEnd.Dispose();
                    await tls.DisposeAsync();
                    tcp.Dispose();
                    stop.Dispose();
                    if (_attachments.TryRemove(busId, out _))
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
        }
        catch
        {
            await tls.DisposeAsync();
            tcp.Dispose();
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

    private EntityRef Device(string busId) => new(busId, NameOf(busId), "device");

    private string NameOf(string busId) => _remote.Devices.FirstOrDefault(d => d.BusId == busId)?.Name ?? busId;

    private Task RaiseAsync() => Changed?.Invoke(Current) ?? Task.CompletedTask;

    /// <param name="Port">The virtual port, or the driver's port for a driver attach; null for another gateway attach.</param>
    /// <param name="Via">Where another gateway attach came from.</param>
    /// <param name="Driver">This client's own attach, made through the driver and the gateway.</param>
    private sealed record Attachment(int? Port, CancellationTokenSource Stop, DateTimeOffset Since, string? Via = null, bool Driver = false);

    [LoggerMessage(Level = LogLevel.Information, Message = "Attached {BusId} on virtual port {Port}")]
    private partial void LogAttached(string busId, int port);

    [LoggerMessage(Level = LogLevel.Information, Message = "Attached {BusId} through the gateway for {From}")]
    private partial void LogGatewayAttached(string busId, string from);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Gateway request for {Subject} failed: {Reason}")]
    private partial void LogGatewayFailed(string subject, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Attaching {BusId} failed: {Reason}")]
    private partial void LogAttachFailed(string busId, string reason);
}
