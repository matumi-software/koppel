using System.Net.Sockets;
using Koppel.Core.Usb;

namespace Koppel.Tests.Usb;

/// <summary>
/// Plays usbip-win2's driver: on attach it connects to the given USB/IP server (Koppel's gateway),
/// imports the device and keeps the connection, as the kernel driver would. The test reads and writes
/// USB traffic on <see cref="Connection"/>.
/// </summary>
internal sealed class FakeUsbIpDriver : IUsbIpDriver, IDisposable
{
    public const int DriverPort = 1;

    private TcpClient? _tcp;
    private int _attaches;

    public void Dispose() => _tcp?.Dispose();

    public string? Unavailable => null;

    public TaskCompletionSource<NetworkStream> Connection { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

    public List<string> Detached { get; } = [];

    /// <summary>How many times Koppel asked this driver to attach.</summary>
    public int Attaches => Volatile.Read(ref _attaches);

    /// <summary>When set, an attach returns only once this completes, as usbip.exe does when it takes a while.</summary>
    public TaskCompletionSource? AttachGate { get; set; }

    /// <summary>When set, a detach returns only once this completes.</summary>
    public TaskCompletionSource? DetachGate { get; set; }

    /// <summary>The device goes away on the network side, as when the link drops.</summary>
    public void DropConnection() => _tcp?.Dispose();

    public async Task<int> AttachAsync(string host, int port, string busId, CancellationToken cancellationToken)
    {
        Interlocked.Increment(ref _attaches);
        _tcp = new TcpClient();
        await _tcp.ConnectAsync(host, port, cancellationToken);
        var stream = _tcp.GetStream();
        await stream.WriteAsync(UsbIpProtocol.ImportRequest(busId), cancellationToken);
        var reply = await UsbIpProtocol.ReadHeaderAsync(stream, cancellationToken);
        if (reply.Status != UsbIpProtocol.StatusOk)
        {
            throw new UsbHostException("Device not available.");
        }

        await UsbIpProtocol.ReadImportedDeviceAsync(stream, cancellationToken);
        Connection.TrySetResult(stream);
        if (AttachGate is { } gate)
        {
            await gate.Task.WaitAsync(cancellationToken);
        }

        return DriverPort;
    }

    public async Task DetachAsync(string host, int port, string busId, int driverPort, CancellationToken cancellationToken)
    {
        Detached.Add($"{busId}@{driverPort}");
        _tcp?.Dispose();
        if (DetachGate is { } gate)
        {
            await gate.Task.WaitAsync(cancellationToken);
        }

        // usbip.exe returns once the device is gone, by which time the gateway session has ended too.
        await Task.Delay(TimeSpan.FromMilliseconds(500), cancellationToken);
    }
}
