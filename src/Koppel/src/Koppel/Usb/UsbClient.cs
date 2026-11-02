using System.Collections.Concurrent;
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
public sealed record ClientDeviceView(DeviceView Device, bool AttachedHere, int? Port, DateTimeOffset? AttachedSince, string? Fault);

/// <summary>What the client's UI shows: the link to the server and the devices it shares.</summary>
/// <param name="Link"><c>unconfigured</c>, <c>connecting</c>, <c>connected</c> or <c>error</c>.</param>
public sealed record ClientDevices(string Link, string? LinkMessage, IReadOnlyList<ClientDeviceView> Devices, string? Unavailable);

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
    TimeProvider time,
    IActivityRecorder activity,
    ILogger<UsbClient> logger) : BackgroundService
{
    private readonly ConcurrentDictionary<string, Attachment> _attachments = new();
    private readonly ConcurrentDictionary<string, string> _faults = new();
    private RemoteDevices _remote = new([], 0);
    private string _link = "unconfigured";
    private string? _linkMessage;
    private CancellationTokenSource? _session;

    public event Func<ClientDevices, Task>? Changed;

    public ClientDevices Current => new(
        _link,
        _linkMessage,
        [.. _remote.Devices.Select(d => new ClientDeviceView(
            d,
            _attachments.ContainsKey(d.BusId),
            _attachments.TryGetValue(d.BusId, out var a) ? a.Port : null,
            _attachments.TryGetValue(d.BusId, out var since) ? since.Since : null,
            _faults.GetValueOrDefault(d.BusId)))],
        attacher.Unavailable);

    /// <summary>Starts attaching in the background. The outcome reaches the UI through <see cref="Changed"/>.</summary>
    public void BeginAttach(string busId)
    {
        _faults.TryRemove(busId, out _);
        _ = Task.Run(async () =>
        {
            try
            {
                await AttachAsync(busId);
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
                attacher.Detach(attachment.Port);
                await activity.RecordAsync(Device(busId), ActivityReason.Detached, ActivityOutcome.Succeeded, $"{NameOf(busId)} was detached");
            }
            catch (UsbHostException ex)
            {
                _faults[busId] = ex.Message;
            }

            await attachment.Stop.CancelAsync();
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

    private async Task AttachAsync(string busId)
    {
        var c = connection.Current;
        if (!c.IsComplete || _remote.UsbPort == 0)
        {
            throw new InvalidOperationException("Not connected to a server.");
        }

        if (attacher.Unavailable is { } reason)
        {
            throw new UsbHostException(reason);
        }

        var server = new Uri(c.ServerUrl!);
        using var tokenHttp = new HttpClient(ServerCertificate.CreatePinnedHandler(c.ServerFingerprint!));
        using var tokens = new KoppelTokenClient(tokenHttp, time);
        var token = await tokens.GetAccessTokenAsync(server, c.ClientId!, c.ClientSecret!, CancellationToken.None);

        var tcp = new TcpClient { NoDelay = true };
        await tcp.ConnectAsync(server.Host, _remote.UsbPort);
        var tls = new SslStream(tcp.GetStream(), leaveInnerStreamOpen: false,
            (_, certificate, _, _) => ServerCertificate.Matches(certificate as System.Security.Cryptography.X509Certificates.X509Certificate2, c.ServerFingerprint));
        try
        {
            await tls.AuthenticateAsClientAsync(server.Host);
            await tls.WriteAsync(Encoding.ASCII.GetBytes($"{UsbIpTlsServer.Preface}{token}\n"));
            var answer = await UsbIpTlsServer.ReadLineAsync(tls, CancellationToken.None);
            if (answer != "OK")
            {
                throw new InvalidOperationException($"The server refused the connection: {answer ?? "no answer"}.");
            }

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

    private sealed record Attachment(int Port, CancellationTokenSource Stop, DateTimeOffset Since);

    [LoggerMessage(Level = LogLevel.Information, Message = "Attached {BusId} on virtual port {Port}")]
    private partial void LogAttached(string busId, int port);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Attaching {BusId} failed: {Reason}")]
    private partial void LogAttachFailed(string busId, string reason);
}
