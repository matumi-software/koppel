using System.Collections.Concurrent;
using System.Net.Sockets;

namespace Koppel.Core.Usb;

/// <summary>
/// Shares the devices of a plain USB/IP server next to Koppel instead of this kernel's: usually
/// usbipd-win on the Windows host that runs Koppel's container. Koppel stays the only door from the
/// network. It lists what that server exports and, on import, relays the client's session to it, so
/// the traffic on the network is still TLS behind a token. Which devices are shared is decided on that
/// server (<c>usbipd bind</c>), not here.
/// </summary>
/// <param name="host">The USB/IP server, such as <c>host.docker.internal</c>.</param>
public sealed class UsbIpUpstreamHost(string host, int port, TimeProvider time) : IUsbHost
{
    private static readonly TimeSpan ListFor = TimeSpan.FromSeconds(1);
    private static readonly TimeSpan RetryAfter = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan ConnectTimeout = TimeSpan.FromSeconds(3);

    private readonly Lock _lock = new();
    private readonly ConcurrentDictionary<string, UsbDevice> _inUse = new();
    private IReadOnlyList<UsbDevice> _listed = [];
    private string? _unavailable;
    private DateTimeOffset _nextList = DateTimeOffset.MinValue;

    public string? Unavailable
    {
        get
        {
            Refresh();
            return _unavailable;
        }
    }

    /// <summary>What the server exports, plus the devices Koppel is relaying, which it may no longer list.</summary>
    public IReadOnlyList<UsbDevice> ListDevices()
    {
        Refresh();
        return [.. _listed.Where(d => !_inUse.ContainsKey(d.BusId))
            .Concat(_inUse.Values)
            .OrderBy(d => d.BusId, StringComparer.Ordinal)];
    }

    /// <summary>The server only lists devices it exports, so everything listed is shared.</summary>
    public bool IsShared(UsbDevice device) => true;

    public void Share(string busId)
    {
        if (ListDevices().All(d => d.BusId != busId))
        {
            throw new UsbHostException($"{busId} is not shared on {host}. On Windows, run 'usbipd bind --busid {busId}' as administrator.");
        }
    }

    public void Unshare(string busId) =>
        throw new UsbHostException($"Stop sharing it on {host}. On Windows, run 'usbipd unbind --busid {busId}' as administrator.");

    public ExportStatus GetStatus(string busId) =>
        _inUse.ContainsKey(busId) ? ExportStatus.Used
        : ListDevices().Any(d => d.BusId == busId) ? ExportStatus.Available
        : ExportStatus.NotExported;

    /// <summary>Imports the device from the server and relays <paramref name="socket"/> to it until either side closes.</summary>
    public void Export(string busId, Socket socket)
    {
        NetworkStream? upstream = null;
        try
        {
            upstream = Connect();
            upstream.Write(UsbIpProtocol.ImportRequest(busId));
            var header = new byte[UsbIpProtocol.HeaderSize];
            upstream.ReadExactly(header);
            var reply = UsbIpProtocol.ParseHeader(header);
            if (reply.Code != UsbIpProtocol.OpRepImport || reply.Status != UsbIpProtocol.StatusOk)
            {
                throw new UsbHostException($"{host} would not export {busId}. Another client may be using it.");
            }

            var device = new byte[UsbIpProtocol.DeviceSize];
            upstream.ReadExactly(device);
            // USB traffic can be quiet for a long time; only the handshake has a deadline.
            upstream.Socket.ReceiveTimeout = 0;
            _inUse[busId] = _listed.FirstOrDefault(d => d.BusId == busId) ?? UsbIpProtocol.ReadDevice(device);
        }
        catch (Exception ex) when (ex is IOException or SocketException or OperationCanceledException)
        {
            upstream?.Dispose();
            throw new UsbHostException($"Could not import {busId} from {host}:{port}: {ex.Message}", ex);
        }
        catch
        {
            upstream?.Dispose();
            throw;
        }

        _ = Task.Run(async () =>
        {
            // Closing the socket here ends the caller's relay when the server hangs up first.
            await using var local = new NetworkStream(socket, ownsSocket: true);
            try
            {
                await UsbSockets.RelayAsync(local, upstream, CancellationToken.None);
            }
            finally
            {
                await upstream.DisposeAsync();
                _inUse.TryRemove(busId, out _);
            }
        });
    }

    private void Refresh()
    {
        lock (_lock)
        {
            if (time.GetUtcNow() < _nextList)
            {
                return;
            }

            try
            {
                using var stream = Connect();
                stream.Write(UsbIpProtocol.DevListRequest());
                var header = new byte[UsbIpProtocol.HeaderSize];
                stream.ReadExactly(header);
                var reply = UsbIpProtocol.ParseHeader(header);
                if (reply.Code != UsbIpProtocol.OpRepDevList || reply.Status != UsbIpProtocol.StatusOk)
                {
                    throw new IOException("it answered the device list request with an error");
                }

                _listed = [.. UsbIpProtocol.ReadDevList(stream).Where(d => !d.IsHub)];
                _unavailable = null;
                _nextList = time.GetUtcNow() + ListFor;
            }
            catch (Exception ex) when (ex is IOException or SocketException or OperationCanceledException)
            {
                _listed = [];
                _unavailable = $"Cannot reach the USB/IP server on {host}:{port} ({ex.Message}). On Windows, install usbipd-win with 'winget install usbipd' and share devices with 'usbipd bind --busid <id>'.";
                _nextList = time.GetUtcNow() + RetryAfter;
            }
        }
    }

    private NetworkStream Connect()
    {
        var socket = new Socket(SocketType.Stream, ProtocolType.Tcp) { NoDelay = true };
        try
        {
            using var timeout = new CancellationTokenSource(ConnectTimeout);
            // Socket.Connect has no timeout, and a firewall that drops packets would hang it for a long time.
            socket.ConnectAsync(host, port, timeout.Token).AsTask().GetAwaiter().GetResult();
            socket.ReceiveTimeout = (int)ConnectTimeout.TotalMilliseconds;
            return new NetworkStream(socket, ownsSocket: true);
        }
        catch
        {
            socket.Dispose();
            throw;
        }
    }
}
