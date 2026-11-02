using System.Collections.Concurrent;
using System.Net.Sockets;

namespace Koppel.Core.Usb;

/// <summary>
/// Shares the devices of a plain USB/IP server next to Koppel instead of this kernel's: usually
/// usbipd-win on the Windows host that runs Koppel's container. Koppel stays the only door from the
/// network. It lists what that server exports and, on export, relays the client's session straight to
/// it, so the traffic on the network is still TLS behind a token. Which devices are shared is decided
/// on that server (<c>usbipd bind</c>), not here.
/// </summary>
/// <remarks>
/// The device list is a cached snapshot. A background refresh with one deadline keeps it fresh, so no
/// caller waits on the network, and nothing here holds a lock while it does I/O. What the server says is
/// not trusted: the device count and the bytes read for the list are capped.
/// </remarks>
/// <param name="host">The USB/IP server, such as <c>host.docker.internal</c>.</param>
public sealed class UsbIpUpstreamHost(string host, int port, TimeProvider time, TimeSpan? refreshDeadline = null) : IUsbHost
{
    /// <summary>The most devices Koppel accepts in one list.</summary>
    public const int MaxDevices = 256;

    /// <summary>The most bytes Koppel reads for one list.</summary>
    public const int MaxListBytes = 512 * 1024;

    private static readonly TimeSpan ListFor = TimeSpan.FromSeconds(1);
    private static readonly TimeSpan RetryAfter = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan DefaultDeadline = TimeSpan.FromSeconds(5);

    private readonly TimeSpan _deadline = refreshDeadline ?? DefaultDeadline;
    private readonly ConcurrentDictionary<string, UsbDevice> _inUse = new();
    private Snapshot? _snapshot;
    private Task _refresh = Task.CompletedTask;

    public string? Unavailable => Volatile.Read(ref _snapshot)?.Unavailable;

    public bool CanShare => false;

    public bool CanUnshare => false;

    public bool CanReset => false;

    /// <summary>The server only lists devices it exports, so everything listed is shared.</summary>
    public bool IsShared(UsbDevice device) => true;

    public string SharingAdvice(string busId, bool share) => share
        ? $"{busId} is not shared on {host}. On Windows, run 'usbipd bind --busid {busId}' as administrator."
        : $"Stop sharing it on {host}. On Windows, run 'usbipd unbind --busid {busId}' as administrator.";

    /// <summary>
    /// What the server exports, plus the devices Koppel is relaying, which it may no longer list. Returns
    /// the cached snapshot and refreshes it in the background; only the very first call waits for the server.
    /// </summary>
    public async Task<IReadOnlyList<UsbDevice>> ListDevicesAsync(CancellationToken cancellationToken)
    {
        var snapshot = Volatile.Read(ref _snapshot);
        if (snapshot is null)
        {
            await StartRefresh().WaitAsync(cancellationToken);
            snapshot = Volatile.Read(ref _snapshot)!;
        }
        else if (time.GetUtcNow() >= snapshot.NextRefresh)
        {
            _ = StartRefresh();
        }

        return [.. snapshot.Devices.Where(d => !_inUse.ContainsKey(d.BusId))
            .Concat(_inUse.Values)
            .OrderBy(d => d.BusId, StringComparer.Ordinal)];
    }

    public Task ShareAsync(string busId, CancellationToken cancellationToken) => throw new NotSupportedException(SharingAdvice(busId, share: true));

    public Task UnshareAsync(string busId, CancellationToken cancellationToken) => throw new NotSupportedException(SharingAdvice(busId, share: false));

    public Task ResetAsync(string busId, CancellationToken cancellationToken) => Task.CompletedTask;

    public async Task<ExportStatus> GetStatusAsync(string busId, CancellationToken cancellationToken) =>
        _inUse.ContainsKey(busId) ? ExportStatus.Used
        : (await ListDevicesAsync(cancellationToken)).Any(d => d.BusId == busId) ? ExportStatus.Available
        : ExportStatus.NotExported;

    /// <summary>Imports the device from the server and relays <paramref name="network"/> to it until either side closes.</summary>
    public async Task ExportAsync(UsbDevice device, Stream network, Func<CancellationToken, Task> exported, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(device);
        ArgumentNullException.ThrowIfNull(network);
        ArgumentNullException.ThrowIfNull(exported);
        var busId = UsbBusId.Require(device.BusId);
        NetworkStream? upstream = null;

        // Claim first, so two imports of one device cannot both connect. The claim is released after the
        // upstream stream is disposed, so a second import never meets a half-closed first one.
        if (!_inUse.TryAdd(busId, device))
        {
            throw new UsbHostException($"{busId} is already being used through this server.");
        }

        try
        {
            using (var handshake = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken))
            {
                handshake.CancelAfter(_deadline);
                upstream = await ConnectAsync(handshake.Token);
                await upstream.WriteAsync(UsbIpProtocol.ImportRequest(busId), handshake.Token);
                var reply = await UsbIpProtocol.ReadHeaderAsync(upstream, handshake.Token);
                if (reply.Code != UsbIpProtocol.OpRepImport || reply.Status != UsbIpProtocol.StatusOk)
                {
                    throw new UsbHostException($"{host} would not export {busId}. Another client may be using it.");
                }

                await UsbIpProtocol.ReadImportedDeviceAsync(upstream, handshake.Token);
            }

            // USB traffic can be quiet for a long time; only the handshake has a deadline.
            await exported(cancellationToken);
            await UsbSockets.RelayAsync(network, upstream, cancellationToken);
        }
        catch (Exception ex) when (ex is IOException or SocketException or InvalidDataException or OperationCanceledException && !cancellationToken.IsCancellationRequested)
        {
            throw new UsbHostException($"Could not import {busId} from {host}:{port}: {ex.Message}", ex);
        }
        finally
        {
            if (upstream is not null)
            {
                await upstream.DisposeAsync();
            }

            _inUse.TryRemove(busId, out _);
        }
    }

    /// <summary>Starts a refresh unless one is running, and returns the one that is.</summary>
    private Task StartRefresh()
    {
        var running = Volatile.Read(ref _refresh);
        if (!running.IsCompleted)
        {
            return running;
        }

        var source = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        if (Interlocked.CompareExchange(ref _refresh, source.Task, running) != running)
        {
            return Volatile.Read(ref _refresh);
        }

        _ = Task.Run(async () =>
        {
            try
            {
                Volatile.Write(ref _snapshot, await ListUpstreamAsync());
            }
            finally
            {
                source.SetResult();
            }
        });
        return source.Task;
    }

    /// <summary>One look at the server, under one deadline for the connect, the request and the whole reply.</summary>
    private async Task<Snapshot> ListUpstreamAsync()
    {
        try
        {
            using var deadline = new CancellationTokenSource(_deadline, time);
            await using var stream = await ConnectAsync(deadline.Token);
            await stream.WriteAsync(UsbIpProtocol.DevListRequest(), deadline.Token);
            var reply = await UsbIpProtocol.ReadHeaderAsync(stream, deadline.Token);
            if (reply.Code != UsbIpProtocol.OpRepDevList || reply.Status != UsbIpProtocol.StatusOk)
            {
                throw new IOException("it answered the device list request with an error");
            }

            var devices = await UsbIpProtocol.ReadDevListAsync(stream, MaxDevices, MaxListBytes, deadline.Token);
            // Bus IDs come from another machine and end up in the UI and the logs.
            return new Snapshot([.. devices.Where(d => !d.IsHub && UsbBusId.IsValid(d.BusId))], null, time.GetUtcNow() + ListFor);
        }
        catch (Exception ex)
        {
            var why = ex is OperationCanceledException ? "it did not answer in time" : ex.Message;
            return new Snapshot([], $"Cannot reach the USB/IP server on {host}:{port} ({why}). On Windows, install usbipd-win with 'winget install usbipd' and share devices with 'usbipd bind --busid <id>'.", time.GetUtcNow() + RetryAfter);
        }
    }

    private async Task<NetworkStream> ConnectAsync(CancellationToken cancellationToken)
    {
        var socket = new Socket(SocketType.Stream, ProtocolType.Tcp) { NoDelay = true };
        try
        {
            await socket.ConnectAsync(host, port, cancellationToken);
            return new NetworkStream(socket, ownsSocket: true);
        }
        catch
        {
            socket.Dispose();
            throw;
        }
    }

    private sealed record Snapshot(IReadOnlyList<UsbDevice> Devices, string? Unavailable, DateTimeOffset NextRefresh);
}
