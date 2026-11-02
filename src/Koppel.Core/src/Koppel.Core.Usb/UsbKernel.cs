using System.Net;
using System.Net.Sockets;

namespace Koppel.Core.Usb;

/// <summary>Whether a shared device is free, in use by a client, or broken (the kernel's usbip_status).</summary>
public enum ExportStatus
{
    NotExported = 0,
    Available = 1,
    Used = 2,
    Error = 3,
}

/// <summary>
/// The server side of USB/IP on this machine: list devices and hand them to the network. What a host
/// can do differs (the Linux kernel shares devices itself; usbipd-win on another machine shares them
/// by its own command), so the interface says what each host can do and callers check before they ask.
/// </summary>
public interface IUsbHost
{
    /// <summary>Null when this machine can share USB devices; otherwise why not, for the UI.</summary>
    string? Unavailable { get; }

    /// <summary>True when <see cref="ShareAsync"/> works: Koppel itself decides which devices are shared.</summary>
    bool CanShare { get; }

    /// <summary>True when <see cref="UnshareAsync"/> works.</summary>
    bool CanUnshare { get; }

    /// <summary>True when <see cref="ResetAsync"/> does something.</summary>
    bool CanReset { get; }

    Task<IReadOnlyList<UsbDevice>> ListDevicesAsync(CancellationToken cancellationToken);

    /// <summary>Whether a listed device is ready to export. The Linux kernel says so by binding it to usbip-host.</summary>
    bool IsShared(UsbDevice device);

    /// <summary>
    /// What to tell the user when this host cannot share or unshare a device itself, such as the command
    /// to run on the machine that does.
    /// </summary>
    string SharingAdvice(string busId, bool share);

    /// <summary>Takes the device from its driver and gives it to usbip-host, so it can be exported. Only where <see cref="CanShare"/>.</summary>
    Task ShareAsync(string busId, CancellationToken cancellationToken);

    /// <summary>Gives the device back to its normal driver. Only where <see cref="CanUnshare"/>.</summary>
    Task UnshareAsync(string busId, CancellationToken cancellationToken);

    Task<ExportStatus> GetStatusAsync(string busId, CancellationToken cancellationToken);

    /// <summary>
    /// Resets a shared device on its USB port, so a client gets it the way a fresh plug-in would. A
    /// driver on this machine may have changed the device before it was shared: Linux's Logitech
    /// receiver driver switches the receiver to a mode only that driver reads, and the mouse then
    /// stays silent on the client. Only where <see cref="CanReset"/>.
    /// </summary>
    Task ResetAsync(string busId, CancellationToken cancellationToken);

    /// <summary>
    /// Exports <paramref name="device"/> over <paramref name="network"/> and carries its USB traffic
    /// until the client goes away or <paramref name="cancellationToken"/> ends. The host owns the relay,
    /// so the returned task completes when the session is over. <paramref name="exported"/> runs once the
    /// device is exported and before any USB traffic flows, which is when the caller answers the
    /// client's import request. Throws <see cref="UsbHostException"/> before that when the host cannot
    /// export the device.
    /// </summary>
    Task ExportAsync(UsbDevice device, Stream network, Func<CancellationToken, Task> exported, CancellationToken cancellationToken);
}

/// <summary>A virtual host controller port on the client.</summary>
public sealed record VhciPort(int Port, string Hub, int Status, uint DevId, string? LocalBusId)
{
    /// <summary>VDEV_ST_NULL: nothing attached.</summary>
    public bool IsFree => Status == 4;
}

/// <summary>The client side of USB/IP on this machine: attach remote devices as local ones.</summary>
public interface IUsbAttacher
{
    /// <summary>Null when this machine can attach USB devices; otherwise why not, for the UI.</summary>
    string? Unavailable { get; }

    IReadOnlyList<VhciPort> Ports();

    /// <summary>
    /// Hands a socket that has already completed OP_REQ_IMPORT to the virtual host controller.
    /// Returns the port used. Keep <paramref name="socket"/> alive until detached.
    /// </summary>
    int Attach(Socket socket, uint devId, UsbSpeed speed);

    void Detach(int port);
}

/// <summary>
/// A USB/IP driver that makes its own connection to a USB/IP server, such as usbip-win2 on Windows.
/// Linux takes a socket from Koppel instead (<see cref="IUsbAttacher"/>). Koppel points this kind of
/// driver at its own gateway on 127.0.0.1, which carries the session to the server over TLS.
/// </summary>
public interface IUsbIpDriver
{
    /// <summary>Null when the driver is installed; otherwise why not, for the UI.</summary>
    string? Unavailable { get; }

    /// <summary>Attaches <paramref name="busId"/> from the USB/IP server at host:port. Returns the driver's port.</summary>
    Task<int> AttachAsync(string host, int port, string busId, CancellationToken cancellationToken);

    /// <summary>Detaches the device on <paramref name="driverPort"/> and stops the driver from attaching it again.</summary>
    Task DetachAsync(string host, int port, string busId, int driverPort, CancellationToken cancellationToken);
}

/// <summary>Plumbing shared by both ends.</summary>
public static class UsbSockets
{
    /// <summary>How long <see cref="CreateLoopbackPairAsync"/> waits for its own connection to arrive.</summary>
    private static readonly TimeSpan PairTimeout = TimeSpan.FromSeconds(5);

    /// <summary>
    /// Two connected loopback TCP sockets. The kernel gets one end as a plain socket; Koppel relays the
    /// other end through TLS. That is how USB/IP traffic stays encrypted on the network. Another process
    /// on the machine may connect to the listener first, so a connection counts only when it comes from
    /// the kernel end's own address; any other is closed and the wait goes on.
    /// </summary>
    public static async Task<(Socket KernelEnd, Socket RelayEnd)> CreateLoopbackPairAsync(CancellationToken cancellationToken)
    {
        using var listener = new Socket(AddressFamily.InterNetwork, SocketType.Stream, ProtocolType.Tcp);
        listener.Bind(new IPEndPoint(IPAddress.Loopback, 0));
        listener.Listen(8);
        var kernelEnd = new Socket(AddressFamily.InterNetwork, SocketType.Stream, ProtocolType.Tcp) { NoDelay = true };
        try
        {
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timeout.CancelAfter(PairTimeout);
            await kernelEnd.ConnectAsync(listener.LocalEndPoint!, timeout.Token);
            while (true)
            {
                var candidate = await listener.AcceptAsync(timeout.Token);
                if (candidate.RemoteEndPoint is { } remote && remote.Equals(kernelEnd.LocalEndPoint))
                {
                    candidate.NoDelay = true;
                    return (kernelEnd, candidate);
                }

                candidate.Dispose();
            }
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            kernelEnd.Dispose();
            throw new IOException("The loopback connection for the kernel never arrived.");
        }
        catch
        {
            kernelEnd.Dispose();
            throw;
        }
    }

    /// <summary>
    /// Has the OS probe an idle connection: after 30 s of silence, every 10 s, and drop it after 3
    /// unanswered probes. A half-open link (a server that lost power, a NAT that forgot the flow) then
    /// ends in about a minute instead of leaving a device attached to nothing.
    /// </summary>
    public static void EnableKeepAlive(Socket socket)
    {
        ArgumentNullException.ThrowIfNull(socket);
        try
        {
            socket.SetSocketOption(SocketOptionLevel.Socket, SocketOptionName.KeepAlive, true);
            socket.SetSocketOption(SocketOptionLevel.Tcp, SocketOptionName.TcpKeepAliveTime, 30);
            socket.SetSocketOption(SocketOptionLevel.Tcp, SocketOptionName.TcpKeepAliveInterval, 10);
            socket.SetSocketOption(SocketOptionLevel.Tcp, SocketOptionName.TcpKeepAliveRetryCount, 3);
        }
        catch (Exception ex) when (ex is SocketException or PlatformNotSupportedException)
        {
            // An OS without these options keeps its own keepalive defaults.
        }
    }

    /// <summary>The socket's file descriptor, which the kernel's sysfs files take as text.</summary>
    public static int FileDescriptor(Socket socket)
    {
        ArgumentNullException.ThrowIfNull(socket);
        return checked((int)socket.SafeHandle.DangerousGetHandle());
    }

    /// <summary>Copies both directions until either side closes, then returns.</summary>
    public static async Task RelayAsync(Stream a, Stream b, CancellationToken cancellationToken)
    {
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        var aToB = a.CopyToAsync(b, linked.Token);
        var bToA = b.CopyToAsync(a, linked.Token);
        await Task.WhenAny(aToB, bToA);
        await linked.CancelAsync();
        try
        {
            await Task.WhenAll(aToB, bToA);
        }
        catch (Exception ex) when (ex is OperationCanceledException or IOException or ObjectDisposedException)
        {
            // One side closed; the other copy was cut off. That is the normal end of a session.
        }
    }
}

/// <summary>Stands in on platforms where Koppel cannot drive USB yet.</summary>
public sealed class UnsupportedUsb(string reason) : IUsbHost, IUsbAttacher, IUsbIpDriver
{
    public Task<int> AttachAsync(string host, int port, string busId, CancellationToken cancellationToken) => throw new UsbHostException(reason);

    public Task DetachAsync(string host, int port, string busId, int driverPort, CancellationToken cancellationToken) => throw new UsbHostException(reason);

    public string? Unavailable => reason;

    public bool CanShare => false;

    public bool CanUnshare => false;

    public bool CanReset => false;

    public Task<IReadOnlyList<UsbDevice>> ListDevicesAsync(CancellationToken cancellationToken) => Task.FromResult<IReadOnlyList<UsbDevice>>([]);

    public bool IsShared(UsbDevice device) => false;

    public string SharingAdvice(string busId, bool share) => reason;

    public Task ShareAsync(string busId, CancellationToken cancellationToken) => throw new UsbHostException(reason);

    public Task UnshareAsync(string busId, CancellationToken cancellationToken) => throw new UsbHostException(reason);

    public Task<ExportStatus> GetStatusAsync(string busId, CancellationToken cancellationToken) => Task.FromResult(ExportStatus.NotExported);

    public Task ResetAsync(string busId, CancellationToken cancellationToken) => Task.CompletedTask;

    public Task ExportAsync(UsbDevice device, Stream network, Func<CancellationToken, Task> exported, CancellationToken cancellationToken) => throw new UsbHostException(reason);

    public IReadOnlyList<VhciPort> Ports() => [];

    public int Attach(Socket socket, uint devId, UsbSpeed speed) => throw new UsbHostException(reason);

    public void Detach(int port) => throw new UsbHostException(reason);
}
