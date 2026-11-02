using System.Collections.Concurrent;
using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using System.Text;
using Koppel.Core.Activity;
using Koppel.Core.Auth;
using Koppel.Core.Configuration;
using Koppel.Core.Usb;
using Koppel.Server;
using Koppel.Settings;
using OpenIddict.Validation;

namespace Koppel.Usb;

/// <summary>Where the USB port stands. <see cref="Error"/> says why it is not listening when it should be.</summary>
internal sealed record UsbListenerStatus(bool Listening, int Port, string? Error);

/// <summary>
/// The encrypted USB data plane (server role). Listens with TLS on the configured port, while the role
/// is server or both. Each connection first proves who it is with a bearer token, then speaks plain
/// USB/IP inside the TLS stream. On import the host relays the stream to the device, so USB traffic
/// never crosses the network unencrypted.
/// </summary>
/// <remarks>
/// <para>
/// The handshake before USB/IP is one line each way: the client sends <c>KOPPEL/1 &lt;access token&gt;\n</c>,
/// the server answers <c>OK\n</c> or <c>DENIED &lt;reason&gt;\n</c> and closes. The client waits for the
/// answer before it sends anything else.
/// </para>
/// <para>
/// Before a client is authenticated it costs the server little: the number of connections in that state is
/// capped, in all and per address, and they have a few seconds to finish. A port change restarts only
/// the listener; the sessions already running keep going. A session ends when its client is revoked or
/// its secret is rotated (see <see cref="LiveSessions"/>), when the role leaves the server side, or when
/// Koppel stops.
/// </para>
/// </remarks>
internal sealed partial class UsbIpTlsServer(
    UsbServer usb,
    ServerSecretsStore secrets,
    LiveSessions live,
    IActivityRecorder activity,
    SettingsStore settings,
    IServiceProvider services,
    ILogger<UsbIpTlsServer> logger) : BackgroundService
{
    public const string Preface = "KOPPEL/1 ";
    private const int MaxPrefaceBytes = 8192;
    private const int MaxPreAuth = 64;
    private const int MaxPreAuthPerAddress = 8;

    /// <summary>Time for the TLS handshake and the token, together.</summary>
    private static readonly TimeSpan PreAuthBudget = TimeSpan.FromSeconds(5);

    /// <summary>Time for an authenticated client to ask for the device list or an import.</summary>
    private static readonly TimeSpan RequestBudget = TimeSpan.FromSeconds(10);

    private static readonly TimeSpan BindRetry = TimeSpan.FromSeconds(10);
    private static readonly TimeSpan StopGrace = TimeSpan.FromSeconds(5);

    private readonly ConcurrentDictionary<Task, byte> _sessions = new();
    private readonly ConcurrentDictionary<IPAddress, int> _preAuthByAddress = new();
    private readonly SemaphoreSlim _wake = new(0);
    private int _preAuth;
    private UsbListenerStatus _status = new(false, 0, null);
    private CancellationTokenSource? _cycle;
    private int _cyclePort;
    private CancellationTokenSource? _serving;

    /// <summary>Raised when the listener starts, stops or fails to bind.</summary>
    public event Func<UsbListenerStatus, Task>? StatusChanged;

    public UsbListenerStatus Status => Volatile.Read(ref _status);

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        settings.Changed += OnSettingsAsync;
        try
        {
            await RunAsync(stoppingToken);
        }
        catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
        {
            // Stopping.
        }
        finally
        {
            settings.Changed -= OnSettingsAsync;
            await EndServingAsync();
        }
    }

    public override void Dispose()
    {
        _wake.Dispose();
        base.Dispose();
    }

    private static bool Serves(KoppelRole role) => role is KoppelRole.Server or KoppelRole.Both;

    private async Task RunAsync(CancellationToken stoppingToken)
    {
        while (!stoppingToken.IsCancellationRequested)
        {
            var current = settings.Current;
            if (!Serves(current.Role))
            {
                // A machine that is only a client has no USB port open, and what it served ends.
                await EndServingAsync();
                await SetStatusAsync(new UsbListenerStatus(false, current.ListenPort, null));
                await _wake.WaitAsync(stoppingToken);
                continue;
            }

            _serving ??= CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
            var cycle = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
            _cyclePort = current.ListenPort;
            _cycle = cycle;
            try
            {
                // A change that came in before the cycle was in place would have nothing to cancel.
                if (settings.Current != current)
                {
                    continue;
                }

                await ListenAsync(current.ListenPort, _serving.Token, cycle.Token);
            }
            catch (OperationCanceledException) when (!stoppingToken.IsCancellationRequested)
            {
                // Port or role changed; loop and listen on the new one, or not at all.
            }
            catch (Exception ex) when (ex is SocketException or InvalidOperationException)
            {
                LogListenFailed(current.ListenPort, ex);
                await SetStatusAsync(new UsbListenerStatus(false, current.ListenPort, $"Cannot listen on port {current.ListenPort}: {ex.Message}"));
                await _wake.WaitAsync(BindRetry, stoppingToken);
            }
            finally
            {
                _cycle = null;
                cycle.Dispose();
            }
        }
    }

    /// <summary>Runs the listener until the cycle ends. Sessions it accepts live on the serving token, not the cycle's.</summary>
    private async Task ListenAsync(int port, CancellationToken serving, CancellationToken cycle)
    {
        using var listener = new Socket(AddressFamily.InterNetworkV6, SocketType.Stream, ProtocolType.Tcp) { DualMode = true };
        listener.Bind(new IPEndPoint(IPAddress.IPv6Any, port));
        listener.Listen(64);
        LogListening(port);
        await SetStatusAsync(new UsbListenerStatus(true, port, null));
        while (true)
        {
            var socket = await listener.AcceptAsync(cycle);
            Track(Task.Run(() => HandleAsync(socket, serving), CancellationToken.None));
        }
    }

    private Task OnSettingsAsync(KoppelSettings next)
    {
        try
        {
            if (_cycle is { } cycle && (!Serves(next.Role) || next.ListenPort != _cyclePort))
            {
                cycle.Cancel();
            }
        }
        catch (ObjectDisposedException)
        {
            // The listener cycle ended by itself.
        }

        _wake.Release();
        return Task.CompletedTask;
    }

    private async Task SetStatusAsync(UsbListenerStatus status)
    {
        if (Interlocked.Exchange(ref _status, status) != status)
        {
            await StatusChanged.RaiseAsync(status, logger);
        }
    }

    private void Track(Task session)
    {
        _sessions[session] = 0;
        _ = session.ContinueWith(done => _sessions.TryRemove(done, out _), TaskScheduler.Default);
    }

    /// <summary>Ends every session and waits a little for them to finish, so nothing outlives the listener's owner.</summary>
    private async Task EndServingAsync()
    {
        if (_serving is { } serving)
        {
            _serving = null;
            await serving.CancelAsync();
            serving.Dispose();
        }

        var running = _sessions.Keys.ToArray();
        if (running.Length == 0)
        {
            return;
        }

        try
        {
            await Task.WhenAll(running).WaitAsync(StopGrace);
        }
        catch (TimeoutException)
        {
            LogSessionsStillRunning(_sessions.Count);
        }
    }

    private async Task HandleAsync(Socket socket, CancellationToken serving)
    {
        var address = AddressOf(socket);
        if (!TryEnterPreAuth(address))
        {
            LogTooManyHandshakes(address);
            socket.Dispose();
            return;
        }

        var preAuth = true;
        try
        {
            Tune(socket);
            await using var tls = new SslStream(new NetworkStream(socket, ownsSocket: true));
            SessionIdentity? identity;
            using (var budget = CancellationTokenSource.CreateLinkedTokenSource(serving))
            {
                budget.CancelAfter(PreAuthBudget);
                await tls.AuthenticateAsServerAsync(
                    new SslServerAuthenticationOptions { ServerCertificate = secrets.Certificate },
                    budget.Token);
                identity = await AuthenticateAsync(tls, budget.Token);
            }

            preAuth = false;
            LeavePreAuth(address);
            if (identity is null)
            {
                return;
            }

            using var abort = CancellationTokenSource.CreateLinkedTokenSource(serving);
            using var registration = live.Add(identity.ClientId, identity.Generation, abort.Cancel);
            await ServeAsync(tls, identity.ClientId, abort.Token);
        }
        catch (OperationCanceledException)
        {
            LogSessionEnded("it was cancelled or ran out of time");
        }
        catch (Exception ex) when (ex is IOException or System.Security.Authentication.AuthenticationException or EndOfStreamException or ObjectDisposedException or SocketException)
        {
            LogSessionEnded(ex.Message);
        }
        catch (Exception ex)
        {
            // One bad session must not take the host down, and it must leave a trace.
            LogSessionFailed(ex);
        }
        finally
        {
            if (preAuth)
            {
                LeavePreAuth(address);
            }
        }
    }

    /// <summary>Reads what the client asks for, and answers it.</summary>
    private async Task ServeAsync(SslStream tls, string clientId, CancellationToken cancellationToken)
    {
        UsbIpProtocol.OpHeader header;
        string? busId = null;
        using (var request = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken))
        {
            request.CancelAfter(RequestBudget);
            header = await UsbIpProtocol.ReadHeaderAsync(tls, request.Token);
            if (header.Code == UsbIpProtocol.OpReqImport)
            {
                busId = await UsbIpProtocol.ReadBusIdAsync(tls, request.Token);
            }
        }

        if (header.Code == UsbIpProtocol.OpReqDevList)
        {
            await tls.WriteAsync(UsbIpProtocol.DevListReply(await usb.ExportableAsync(cancellationToken)), cancellationToken);
        }
        else if (busId is not null)
        {
            await ImportAsync(tls, busId, clientId, cancellationToken);
        }
    }

    /// <summary>Reads the preface, validates the token and checks the client is still registered, at the generation the token was issued for.</summary>
    private async Task<SessionIdentity?> AuthenticateAsync(SslStream tls, CancellationToken cancellationToken)
    {
        var line = await ReadPrefaceAsync(tls, cancellationToken);
        if (line is null || !line.StartsWith(Preface, StringComparison.Ordinal))
        {
            await DenyAsync(tls, "expected a Koppel handshake", cancellationToken);
            return null;
        }

        var token = line[Preface.Length..];
        if (token.Length == 0)
        {
            await DenyAsync(tls, "invalid token", cancellationToken);
            return null;
        }

        System.Security.Claims.ClaimsPrincipal principal;
        try
        {
            var validation = services.GetRequiredService<OpenIddictValidationService>();
            principal = await validation.ValidateAccessTokenAsync(token, cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            // Whatever is wrong with the token, the client is told and nothing else about why.
            await DenyAsync(tls, "invalid token", cancellationToken);
            return null;
        }

        if (!secrets.IsCurrent(principal))
        {
            await DenyAsync(tls, "client is not registered", cancellationToken);
            return null;
        }

        await tls.WriteAsync("OK\n"u8.ToArray(), cancellationToken);
        return new SessionIdentity(principal.GetClientId()!, principal.GetGeneration()!.Value);
    }

    private async Task ImportAsync(SslStream tls, string busId, string clientId, CancellationToken cancellationToken)
    {
        var host = usb.Host;
        var device = UsbBusId.IsValid(busId) ? (await usb.ExportableAsync(cancellationToken)).FirstOrDefault(d => d.BusId == busId) : null;
        if (device is null || await host.GetStatusAsync(busId, cancellationToken) != ExportStatus.Available)
        {
            await tls.WriteAsync(UsbIpProtocol.ImportReply(null), cancellationToken);
            return;
        }

        if (host.CanReset)
        {
            try
            {
                await host.ResetAsync(busId, cancellationToken);
            }
            catch (UsbHostException ex)
            {
                // Most devices work without it, so the attach goes ahead.
                LogResetFailed(busId, ex.Message);
            }
        }

        var clientName = secrets.NameOf(clientId) ?? clientId;
        var deviceName = device.Product ?? busId;
        var attached = false;
        try
        {
            // Returns when the client detaches or the link drops; the host then frees the device.
            await host.ExportAsync(device, tls, async token =>
            {
                await tls.WriteAsync(UsbIpProtocol.ImportReply(device), token);
                attached = true;
                LogAttached(busId, clientId);
                await activity.RecordAsync(new EntityRef(busId, deviceName, "device"), ActivityReason.Attached, ActivityOutcome.Succeeded, $"{clientName} attached {deviceName}", clientId);
                await usb.SetAttachedAsync(busId, clientId);
            }, cancellationToken);
        }
        catch (UsbHostException ex) when (!attached)
        {
            LogExportFailed(busId, ex.Message);
            await tls.WriteAsync(UsbIpProtocol.ImportReply(null), CancellationToken.None);
        }
        finally
        {
            if (attached)
            {
                LogDetached(busId, clientId);
                await activity.RecordAsync(new EntityRef(busId, deviceName, "device"), ActivityReason.Detached, ActivityOutcome.Succeeded, $"{clientName} released {deviceName}", clientId);
                await usb.SetAttachedAsync(busId, null);
            }
        }
    }

    private static async Task DenyAsync(SslStream tls, string reason, CancellationToken cancellationToken) =>
        await tls.WriteAsync(Encoding.ASCII.GetBytes($"DENIED {reason}\n"), cancellationToken);

    /// <summary>
    /// Reads the preface line in chunks. The client waits for the answer before it sends anything else,
    /// so a line followed by more bytes is not a Koppel handshake.
    /// </summary>
    private static async Task<string?> ReadPrefaceAsync(Stream stream, CancellationToken cancellationToken)
    {
        var buffer = new byte[MaxPrefaceBytes];
        var length = 0;
        while (length < buffer.Length)
        {
            var read = await stream.ReadAsync(buffer.AsMemory(length), cancellationToken);
            if (read == 0)
            {
                return null;
            }

            var newline = buffer.AsSpan(length, read).IndexOf((byte)'\n');
            length += read;
            if (newline >= 0)
            {
                var end = length - read + newline;
                return end + 1 == length ? Encoding.ASCII.GetString(buffer, 0, end) : null;
            }
        }

        return null;
    }

    /// <summary>Reads one line byte by byte, so nothing after it is consumed. The client side uses it for the server's answer.</summary>
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

    private static IPAddress AddressOf(Socket socket) =>
        socket.RemoteEndPoint is IPEndPoint { Address: var address }
            ? address.IsIPv4MappedToIPv6 ? address.MapToIPv4() : address
            : IPAddress.None;

    private bool TryEnterPreAuth(IPAddress address)
    {
        if (Interlocked.Increment(ref _preAuth) > MaxPreAuth)
        {
            Interlocked.Decrement(ref _preAuth);
            return false;
        }

        if (_preAuthByAddress.AddOrUpdate(address, 1, (_, count) => count + 1) > MaxPreAuthPerAddress)
        {
            LeavePreAuth(address);
            return false;
        }

        return true;
    }

    private void LeavePreAuth(IPAddress address)
    {
        Interlocked.Decrement(ref _preAuth);
        if (_preAuthByAddress.AddOrUpdate(address, 0, (_, count) => count - 1) <= 0)
        {
            _preAuthByAddress.TryRemove(new KeyValuePair<IPAddress, int>(address, 0));
        }
    }

    /// <summary>
    /// Low latency, and TCP keepalive so a client that vanished (a sleeping laptop, a pulled cable) is noticed
    /// in about a minute and the device is freed, instead of staying in use until Koppel restarts.
    /// </summary>
    private static void Tune(Socket socket)
    {
        socket.NoDelay = true;
        UsbSockets.EnableKeepAlive(socket);
    }

    private sealed record SessionIdentity(string ClientId, int Generation);

    [LoggerMessage(Level = LogLevel.Information, Message = "USB data plane listening with TLS on port {Port}")]
    private partial void LogListening(int port);

    [LoggerMessage(Level = LogLevel.Error, Message = "Could not listen on port {Port}")]
    private partial void LogListenFailed(int port, Exception exception);

    [LoggerMessage(Level = LogLevel.Information, Message = "Device {BusId} attached by {ClientId}")]
    private partial void LogAttached(string busId, string clientId);

    [LoggerMessage(Level = LogLevel.Information, Message = "Device {BusId} released by {ClientId}")]
    private partial void LogDetached(string busId, string clientId);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not reset {BusId} before export: {Reason}")]
    private partial void LogResetFailed(string busId, string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Export of {BusId} failed: {Reason}")]
    private partial void LogExportFailed(string busId, string reason);

    [LoggerMessage(Level = LogLevel.Debug, Message = "USB session ended: {Reason}")]
    private partial void LogSessionEnded(string reason);

    [LoggerMessage(Level = LogLevel.Warning, Message = "A USB session failed")]
    private partial void LogSessionFailed(Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Refused a connection from {Address}: too many handshakes are waiting")]
    private partial void LogTooManyHandshakes(IPAddress address);

    [LoggerMessage(Level = LogLevel.Warning, Message = "{Count} USB sessions were still running when the listener stopped")]
    private partial void LogSessionsStillRunning(int count);
}
