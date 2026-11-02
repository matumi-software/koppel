using System.Collections.Concurrent;
using System.Net.Sockets;
using Koppel.Core.Usb;

namespace Koppel.Tests.Usb;

/// <summary>
/// Stands in for the Linux kernel on both ends. The "server kernel" echoes whatever arrives on an
/// exported socket, the way usbip-host would answer URBs. The "client kernel" hands the attached
/// socket to the test, which plays the USB driver.
/// </summary>
internal sealed class FakeKernel : IUsbHost, IUsbAttacher
{
    public static readonly UsbDevice Dongle = new(
        BusId: "1-1.4", Path: "/sys/devices/fake/1-1.4", BusNum: 1, DevNum: 5, Speed: UsbSpeed.Full,
        VendorId: 0x064f, ProductId: 0x2af9, BcdDevice: 0x0100, Class: 0, SubClass: 0, Protocol: 0,
        ConfigurationValue: 1, NumConfigurations: 1, Interfaces: [new UsbInterface(0xff, 0, 0)],
        Manufacturer: "WIBU-SYSTEMS AG", Product: "CodeMeter-Stick", Serial: null, Driver: "usbip-host");

    private readonly ConcurrentDictionary<int, Socket> _attached = new();
    private volatile bool _used;

    public TaskCompletionSource<Socket> AttachedSocket { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

    public string? Unavailable => null;

    public IReadOnlyList<UsbDevice> ListDevices() => [Dongle];

    public void Share(string busId)
    {
    }

    public void Unshare(string busId)
    {
    }

    public ExportStatus GetStatus(string busId) => _used ? ExportStatus.Used : ExportStatus.Available;

    public void Export(string busId, Socket socket)
    {
        _used = true;
        _ = Task.Run(async () =>
        {
            var buffer = new byte[4096];
            try
            {
                int read;
                while ((read = await socket.ReceiveAsync(buffer)) > 0)
                {
                    await socket.SendAsync(buffer.AsMemory(0, read));
                }
            }
            catch (Exception ex) when (ex is SocketException or ObjectDisposedException)
            {
                // Session ended.
            }
            finally
            {
                _used = false;
            }
        });
    }

    public IReadOnlyList<VhciPort> Ports() => [];

    public int Attach(Socket socket, uint devId, UsbSpeed speed)
    {
        _attached[8] = socket;
        AttachedSocket.TrySetResult(socket);
        return 8;
    }

    public void Detach(int port)
    {
        if (_attached.TryRemove(port, out var socket))
        {
            socket.Dispose();
        }
    }
}
