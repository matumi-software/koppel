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

    public void Dispose() => _tcp?.Dispose();

    public string? Unavailable => null;

    public TaskCompletionSource<NetworkStream> Connection { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

    public List<string> Detached { get; } = [];

    public async Task<int> AttachAsync(string host, int port, string busId, CancellationToken cancellationToken)
    {
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
        return DriverPort;
    }

    public Task DetachAsync(string host, int port, string busId, int driverPort, CancellationToken cancellationToken)
    {
        Detached.Add($"{busId}@{driverPort}");
        _tcp?.Dispose();
        return Task.CompletedTask;
    }
}
