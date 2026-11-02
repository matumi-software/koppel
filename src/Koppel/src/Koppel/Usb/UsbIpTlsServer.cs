using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using System.Text;
using Koppel.Core.Activity;
using Koppel.Core.Auth;
using Koppel.Core.Usb;
using Koppel.Server;
using Koppel.Settings;
using OpenIddict.Validation;

namespace Koppel.Usb;

/// <summary>
/// The encrypted USB data plane (server role). Listens with TLS on the configured port. Each
/// connection first proves who it is with a bearer token, then speaks plain USB/IP inside the TLS
/// stream. On import, the kernel gets one end of a loopback socket pair and Koppel relays the other
/// end through TLS, so USB traffic never crosses the network unencrypted.
/// </summary>
/// <remarks>
/// The handshake before USB/IP is one line each way: the client sends <c>KOPPEL/1 &lt;access token&gt;\n</c>,
/// the server answers <c>OK\n</c> or <c>DENIED &lt;reason&gt;\n</c> and closes.
/// </remarks>
internal sealed partial class UsbIpTlsServer(
    UsbServer usb,
    ServerSecretsStore secrets,
    IActivityRecorder activity,
    SettingsStore settings,
    IServiceProvider services,
    ILogger<UsbIpTlsServer> logger) : BackgroundService
{
    public const string Preface = "KOPPEL/1 ";
    private const int MaxPrefaceBytes = 8192;
    private static readonly TimeSpan HandshakeTimeout = TimeSpan.FromSeconds(15);

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        while (!stoppingToken.IsCancellationRequested)
        {
            // Restart the listener when the port setting changes.
            var port = settings.Current.ListenPort;
            using var portChanged = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
            Task OnSettings(KoppelSettings s)
            {
                if (s.ListenPort != port)
                {
                    portChanged.Cancel();
                }

                return Task.CompletedTask;
            }

            settings.Changed += OnSettings;
            try
            {
                await ListenAsync(port, portChanged.Token);
            }
            catch (OperationCanceledException) when (!stoppingToken.IsCancellationRequested)
            {
                // Port changed; loop and listen on the new one.
            }
            catch (SocketException ex)
            {
                LogListenFailed(port, ex);
                await Task.Delay(TimeSpan.FromSeconds(10), stoppingToken);
            }
            finally
            {
                settings.Changed -= OnSettings;
            }
        }
    }

    private async Task ListenAsync(int port, CancellationToken cancellationToken)
    {
        using var listener = new Socket(AddressFamily.InterNetworkV6, SocketType.Stream, ProtocolType.Tcp) { DualMode = true };
        listener.Bind(new IPEndPoint(IPAddress.IPv6Any, port));
        listener.Listen(16);
        LogListening(port);
        while (true)
        {
            var socket = await listener.AcceptAsync(cancellationToken);
            _ = Task.Run(() => HandleAsync(socket, cancellationToken), CancellationToken.None);
        }
    }

    private async Task HandleAsync(Socket socket, CancellationToken stoppingToken)
    {
        socket.NoDelay = true;
        await using var tls = new SslStream(new NetworkStream(socket, ownsSocket: true));
        try
        {
            using (var handshake = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken))
            {
                handshake.CancelAfter(HandshakeTimeout);
                await tls.AuthenticateAsServerAsync(
                    new SslServerAuthenticationOptions { ServerCertificate = secrets.Certificate },
                    handshake.Token);

                var clientId = await AuthenticateAsync(tls, handshake.Token);
                if (clientId is null)
                {
                    return;
                }

                var header = await UsbIpProtocol.ReadHeaderAsync(tls, handshake.Token);
                if (header.Code == UsbIpProtocol.OpReqDevList)
                {
                    await tls.WriteAsync(UsbIpProtocol.DevListReply(usb.Exportable()), handshake.Token);
                    return;
                }

                if (header.Code != UsbIpProtocol.OpReqImport)
                {
                    return;
                }

                var busId = await UsbIpProtocol.ReadBusIdAsync(tls, handshake.Token);
                await ImportAsync(tls, busId, clientId, stoppingToken);
            }
        }
        catch (Exception ex) when (ex is IOException or System.Security.Authentication.AuthenticationException or OperationCanceledException or EndOfStreamException)
        {
            LogSessionEnded(ex.Message);
        }
    }

    /// <summary>Reads the preface, validates the token and checks the client is still registered.</summary>
    private async Task<string?> AuthenticateAsync(SslStream tls, CancellationToken cancellationToken)
    {
        var line = await ReadLineAsync(tls, cancellationToken);
        if (line is null || !line.StartsWith(Preface, StringComparison.Ordinal))
        {
            await DenyAsync(tls, "expected a Koppel handshake", cancellationToken);
            return null;
        }

        try
        {
            var validation = services.GetRequiredService<OpenIddictValidationService>();
            var principal = await validation.ValidateAccessTokenAsync(line[Preface.Length..], cancellationToken);
            var clientId = principal.GetClientId();
            if (!secrets.IsRegistered(clientId))
            {
                await DenyAsync(tls, "client is not registered", cancellationToken);
                return null;
            }

            await tls.WriteAsync("OK\n"u8.ToArray(), cancellationToken);
            return clientId;
        }
        catch (OpenIddict.Abstractions.OpenIddictExceptions.ProtocolException)
        {
            await DenyAsync(tls, "invalid token", cancellationToken);
            return null;
        }
    }

    private async Task ImportAsync(SslStream tls, string busId, string clientId, CancellationToken stoppingToken)
    {
        var device = usb.Exportable().FirstOrDefault(d => d.BusId == busId);
        if (device is null || usb.Host.GetStatus(busId) != ExportStatus.Available)
        {
            await tls.WriteAsync(UsbIpProtocol.ImportReply(null), stoppingToken);
            return;
        }

        var (kernelEnd, relayEnd) = await UsbSockets.CreateLoopbackPairAsync(stoppingToken);
        using (kernelEnd)
        await using (var relay = new NetworkStream(relayEnd, ownsSocket: true))
        {
            try
            {
                usb.Host.Export(busId, kernelEnd);
            }
            catch (UsbHostException ex)
            {
                LogExportFailed(busId, ex.Message);
                await tls.WriteAsync(UsbIpProtocol.ImportReply(null), stoppingToken);
                return;
            }

            await tls.WriteAsync(UsbIpProtocol.ImportReply(device), stoppingToken);
            LogAttached(busId, clientId);
            var clientName = secrets.NameOf(clientId) ?? clientId;
            var deviceName = device.Product ?? busId;
            await activity.RecordAsync(new EntityRef(busId, deviceName, "device"), ActivityReason.Attached, ActivityOutcome.Succeeded, $"{clientName} attached {deviceName}", clientId);
            await usb.SetAttachedAsync(busId, clientId);
            try
            {
                // Runs until the client detaches or the link drops; the kernel then frees the device.
                await UsbSockets.RelayAsync(tls, relay, stoppingToken);
            }
            finally
            {
                LogDetached(busId, clientId);
                await activity.RecordAsync(new EntityRef(busId, deviceName, "device"), ActivityReason.Detached, ActivityOutcome.Succeeded, $"{clientName} released {deviceName}", clientId);
                await usb.SetAttachedAsync(busId, null);
            }
        }
    }

    private static async Task DenyAsync(SslStream tls, string reason, CancellationToken cancellationToken) =>
        await tls.WriteAsync(Encoding.ASCII.GetBytes($"DENIED {reason}\n"), cancellationToken);

    /// <summary>Reads one line byte by byte, so nothing after it is consumed.</summary>
    internal static async Task<string?> ReadLineAsync(Stream stream, CancellationToken cancellationToken)
    {
        var buffer = new byte[MaxPrefaceBytes];
        var single = new byte[1];
        for (var length = 0; length < buffer.Length; length++)
        {
            if (await stream.ReadAsync(single, cancellationToken) == 0)
            {
                return null;
            }

            if (single[0] == (byte)'\n')
            {
                return Encoding.ASCII.GetString(buffer, 0, length);
            }

            buffer[length] = single[0];
        }

        return null;
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "USB data plane listening with TLS on port {Port}")]
    private partial void LogListening(int port);

    [LoggerMessage(Level = LogLevel.Error, Message = "Could not listen on port {Port}")]
    private partial void LogListenFailed(int port, Exception exception);

    [LoggerMessage(Level = LogLevel.Information, Message = "Device {BusId} attached by {ClientId}")]
    private partial void LogAttached(string busId, string clientId);

    [LoggerMessage(Level = LogLevel.Information, Message = "Device {BusId} released by {ClientId}")]
    private partial void LogDetached(string busId, string clientId);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Export of {BusId} failed: {Reason}")]
    private partial void LogExportFailed(string busId, string reason);

    [LoggerMessage(Level = LogLevel.Debug, Message = "USB session ended: {Reason}")]
    private partial void LogSessionEnded(string reason);
}
