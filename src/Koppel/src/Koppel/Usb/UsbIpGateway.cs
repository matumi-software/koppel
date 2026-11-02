using System.Net;
using System.Net.Sockets;
using Koppel.Core.Usb.Windows;
using Koppel.Settings;

namespace Koppel.Usb;

/// <summary>Decides which connections the gateway serves.</summary>
internal interface IGatewayPeerCheck
{
    /// <summary>Whether <paramref name="accepted"/>, a connection the gateway just accepted on <paramref name="gateway"/>, may be served.</summary>
    Task<bool> AllowsAsync(Socket accepted, IPEndPoint gateway, CancellationToken cancellationToken);
}

/// <summary>Serves everyone who reaches the gateway.</summary>
internal sealed class AnyPeer : IGatewayPeerCheck
{
    public Task<bool> AllowsAsync(Socket accepted, IPEndPoint gateway, CancellationToken cancellationToken) => Task.FromResult(true);
}

/// <summary>
/// Serves only connections the Windows kernel owns, which is how usbip-win2's driver connects. Any
/// other process on the machine that reaches the loopback port is turned away.
/// </summary>
internal sealed class KernelPeer(KernelPeerCheck check) : IGatewayPeerCheck
{
    public Task<bool> AllowsAsync(Socket accepted, IPEndPoint gateway, CancellationToken cancellationToken) =>
        accepted.RemoteEndPoint is IPEndPoint peer && peer.AddressFamily == AddressFamily.InterNetwork
            ? check.IsKernelAsync(peer, gateway, cancellationToken)
            : Task.FromResult(false);
}

/// <summary>
/// Client role: a plain USB/IP server for a USB/IP driver on this machine, such as usbip-win2. Koppel
/// on Windows drives that driver itself and points it here; Koppel in a container on Windows leaves it
/// to the host's <c>usbip attach</c>. Off unless configured, except on Windows. Each connection is
/// carried to the server through <see cref="UsbClient"/>'s TLS session, so the network only sees
/// encrypted traffic. It runs only while the role is Client or Both.
/// </summary>
/// <remarks>
/// Anyone who reaches this port gets this client's access to the server, with no login. It listens on
/// <c>Gateway:Address</c> (127.0.0.1 unless set) and <c>Gateway:Port</c>. In a container, listen on all
/// addresses and publish the port on the host's 127.0.0.1 only. On native Windows with a loopback
/// address, only connections the kernel owns (PID 4, the driver) are served. A port that cannot be
/// bound is tried again with a growing wait, and the reason shows in the client's device list.
/// </remarks>
internal sealed partial class UsbIpGateway(
    UsbClient client,
    SettingsStore settings,
    IConfiguration configuration,
    ILogger<UsbIpGateway> logger,
    IGatewayPeerCheck? peerCheck = null) : BackgroundService
{
    /// <summary>On Windows the gateway is how Koppel's own Attach reaches the driver, so it is on by default.</summary>
    public const int WindowsDefaultPort = 3242;

    private static readonly TimeSpan MinBackoff = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan MaxBackoff = TimeSpan.FromSeconds(60);

    private readonly BackgroundTasks _tasks = new(logger);
    private TaskCompletionSource _roleChanged = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private volatile bool _running;

    private bool Wanted => settings.Current.Role != KoppelRole.Server;

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        var port = configuration.GetValue("Gateway:Port", OperatingSystem.IsWindows() ? WindowsDefaultPort : 0);
        if (port == 0)
        {
            return;
        }

        if (!IPAddress.TryParse(configuration["Gateway:Address"] ?? "127.0.0.1", out var address))
        {
            LogBadAddress(configuration["Gateway:Address"]);
            await client.SetGatewayAsync(null, $"Gateway:Address \"{configuration["Gateway:Address"]}\" is not an IP address.");
            return;
        }

        var check = peerCheck ?? ChoosePeerCheck(address);
        Task OnSettings(KoppelSettings _)
        {
            // Wake the loop when the role starts or stops wanting the gateway.
            if (Wanted != _running)
            {
                Interlocked.Exchange(ref _roleChanged, new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously)).TrySetResult();
            }

            return Task.CompletedTask;
        }

        settings.Changed += OnSettings;
        var backoff = MinBackoff;
        try
        {
            while (!stoppingToken.IsCancellationRequested)
            {
                var signal = _roleChanged.Task;
                using var iteration = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
                _ = signal.ContinueWith(_ => Cancel(iteration), TaskScheduler.Default);
                try
                {
                    if (!Wanted)
                    {
                        await client.SetGatewayAsync(null);
                        await Task.Delay(Timeout.Infinite, iteration.Token);
                    }

                    _running = true;
                    if (!Wanted)
                    {
                        continue;
                    }

                    await ListenAsync(address, port, check, iteration.Token);
                }
                catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
                {
                    break;
                }
                catch (OperationCanceledException)
                {
                    // The role changed; look again.
                    backoff = MinBackoff;
                }
                catch (Exception ex)
                {
                    // A port that is taken or reserved, or anything else: the host keeps running, the UI says why.
                    LogListenFailed(address, port, ex);
                    try
                    {
                        await client.SetGatewayAsync(null, $"The gateway could not listen on {address}:{port}: {ex.Message}");
                        await Task.Delay(backoff, iteration.Token);
                    }
                    catch (OperationCanceledException)
                    {
                        // Stopping, or the role changed.
                    }

                    backoff = TimeSpan.FromTicks(Math.Min(backoff.Ticks * 2, MaxBackoff.Ticks));
                }
                finally
                {
                    _running = false;
                }
            }
        }
        finally
        {
            settings.Changed -= OnSettings;
            await client.SetGatewayAsync(null);
        }
    }

    private static void Cancel(CancellationTokenSource source)
    {
        try
        {
            source.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // The iteration ended first.
        }
    }

    private async Task ListenAsync(IPAddress address, int port, IGatewayPeerCheck check, CancellationToken cancellationToken)
    {
        using var listener = new Socket(address.AddressFamily, SocketType.Stream, ProtocolType.Tcp);
        listener.Bind(new IPEndPoint(address, port));
        listener.Listen(16);
        LogListening(address, port);
        await client.SetGatewayAsync(port);
        var gateway = new IPEndPoint(address, port);
        try
        {
            while (true)
            {
                var socket = await listener.AcceptAsync(cancellationToken);
                socket.NoDelay = true;
                _tasks.Run("gateway session", () => ServeAsync(socket, gateway, check, cancellationToken));
            }
        }
        finally
        {
            await client.SetGatewayAsync(null);
        }
    }

    private async Task ServeAsync(Socket socket, IPEndPoint gateway, IGatewayPeerCheck check, CancellationToken cancellationToken)
    {
        try
        {
            if (!await check.AllowsAsync(socket, gateway, cancellationToken))
            {
                LogRejected(socket.RemoteEndPoint?.ToString() ?? "unknown");
                socket.Dispose();
                return;
            }

            await client.ServeGatewayAsync(socket, cancellationToken);
        }
        catch (Exception ex) when (ex is IOException or SocketException or OperationCanceledException or ObjectDisposedException)
        {
            LogSessionEnded(ex.Message);
        }
    }

    /// <summary>
    /// Native Windows with a loopback gateway: only the kernel may connect. Elsewhere, such as the Linux
    /// container of Docker Desktop, the connection arrives from outside this machine's process table, so
    /// the check does not apply and the gateway's port publishing is what keeps it local.
    /// </summary>
    private IGatewayPeerCheck ChoosePeerCheck(IPAddress address)
    {
        if (OperatingSystem.IsWindows() && IPAddress.IsLoopback(address) && Environment.GetEnvironmentVariable("DOTNET_RUNNING_IN_CONTAINER") != "true")
        {
            if (address.AddressFamily != AddressFamily.InterNetwork)
            {
                LogCannotCheck(address);
            }

            return new KernelPeer(new KernelPeerCheck(new WindowsTcpTable()));
        }

        LogNoPeerCheck(address);
        return new AnyPeer();
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "USB/IP gateway listening on {Address}:{Port}")]
    private partial void LogListening(IPAddress address, int port);

    [LoggerMessage(Level = LogLevel.Warning, Message = "The USB/IP gateway could not listen on {Address}:{Port}; it will try again")]
    private partial void LogListenFailed(IPAddress address, int port, Exception exception);

    [LoggerMessage(Level = LogLevel.Error, Message = "Gateway:Address {Address} is not an IP address; the gateway is off")]
    private partial void LogBadAddress(string? address);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Refused a gateway connection from {Peer}: it is not the USB/IP driver (the kernel)")]
    private partial void LogRejected(string peer);

    [LoggerMessage(Level = LogLevel.Information, Message = "The gateway on {Address} accepts every connection: the check for the driver's process only applies to a loopback address on Windows outside a container")]
    private partial void LogNoPeerCheck(IPAddress address);

    [LoggerMessage(Level = LogLevel.Warning, Message = "The gateway on {Address} cannot tell the driver from other processes on an IPv6 address; it refuses every connection. Use 127.0.0.1")]
    private partial void LogCannotCheck(IPAddress address);

    [LoggerMessage(Level = LogLevel.Debug, Message = "Gateway session ended: {Reason}")]
    private partial void LogSessionEnded(string reason);
}
