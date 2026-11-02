using System.Collections.Concurrent;
using System.Net.Sockets;
using Koppel.Core.Usb;

namespace Koppel.Tests.Usb;

/// <summary>
/// Stands in for the Linux kernel on both ends. The "server kernel" echoes whatever arrives on an
/// exported stream, the way usbip-host would answer URBs. The "client kernel" hands the attached
/// socket to the test, which plays the USB driver. The server side is stateful: sharing binds the device
/// to usbip-host, unsharing gives it back, and a test can make either refuse.
/// </summary>
internal sealed class FakeKernel : IUsbHost, IUsbAttacher
{
    public static readonly UsbDevice Dongle = new(
        BusId: "1-1.4", Path: "/sys/devices/fake/1-1.4", BusNum: 1, DevNum: 5, Speed: UsbSpeed.Full,
        VendorId: 0x064f, ProductId: 0x2af9, BcdDevice: 0x0100, Class: 0, SubClass: 0, Protocol: 0,
        ConfigurationValue: 1, NumConfigurations: 1, Interfaces: [new UsbInterface(0xff, 0, 0)],
        Manufacturer: "WIBU-SYSTEMS AG", Product: "CodeMeter-Stick", Serial: "SERIAL-123", Driver: "usbip-host");

    private readonly ConcurrentDictionary<int, Socket> _attached = new();
    private readonly object _state = new();
    private readonly HashSet<string> _bound = ["1-1.4"];
    private volatile bool _used;
    private volatile bool _reset;
    private int _attaches;
    private IReadOnlyList<UsbDevice> _devices = [Dongle];

    public TaskCompletionSource<Socket> AttachedSocket { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

    /// <summary>The socket of the second attach, once the client attached the device again.</summary>
    public TaskCompletionSource<Socket> Reattached { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

    /// <summary>Whether the device was reset before the last export.</summary>
    public bool ResetBeforeExport { get; private set; }

    /// <summary>When set, sharing fails with this message.</summary>
    public string? ShareFailure { get; set; }

    /// <summary>Bus IDs passed to <see cref="UnshareAsync"/>, in order.</summary>
    public List<string> Unshared { get; } = [];

    /// <summary>Bus IDs passed to <see cref="ShareAsync"/>, in order.</summary>
    public List<string> Shared { get; } = [];

    public string? Unavailable => null;

    public bool CanShare => true;

    public bool CanUnshare => true;

    public bool CanReset => true;

    /// <summary>What the server kernel lists. Set it to change what the server offers; a device keeps its binding by bus ID.</summary>
    public IReadOnlyList<UsbDevice> Devices
    {
        get
        {
            lock (_state)
            {
                return _devices;
            }
        }
        set
        {
            lock (_state)
            {
                _devices = value;
            }
        }
    }

    /// <summary>Replaces the devices on the bus.</summary>
    public void Plug(params UsbDevice[] devices) => Devices = devices;

    /// <summary>While set, a reset blocks, so the server stalls an import after the client authenticated.</summary>
    public ManualResetEventSlim? ResetGate { get; set; }

    /// <summary>How many times the client kernel attached a device.</summary>
    public int Attaches => _attaches;

    /// <summary>The ports the client kernel detached, in order.</summary>
    public ConcurrentQueue<int> DetachedPorts { get; } = new();

    /// <summary>Binds or unbinds a device behind Koppel's back, as a person with the usbip tool could.</summary>
    public void Bind(string busId, bool bound)
    {
        lock (_state)
        {
            _ = bound ? _bound.Add(busId) : _bound.Remove(busId);
        }
    }

    public bool IsBound(string busId)
    {
        lock (_state)
        {
            return _bound.Contains(busId);
        }
    }

    public Task<IReadOnlyList<UsbDevice>> ListDevicesAsync(CancellationToken cancellationToken)
    {
        lock (_state)
        {
            return Task.FromResult<IReadOnlyList<UsbDevice>>([.. _devices.Select(d => d with { Driver = _bound.Contains(d.BusId) ? "usbip-host" : "ftdi_sio" })]);
        }
    }

    public bool IsShared(UsbDevice device) => device.Driver == "usbip-host";

    public string SharingAdvice(string busId, bool share) => "Koppel shares devices itself.";

    public Task ShareAsync(string busId, CancellationToken cancellationToken)
    {
        if (ShareFailure is { } failure)
        {
            throw new UsbHostException(failure);
        }

        lock (_state)
        {
            Shared.Add(busId);
            _bound.Add(busId);
        }

        return Task.CompletedTask;
    }

    public Task UnshareAsync(string busId, CancellationToken cancellationToken)
    {
        lock (_state)
        {
            Unshared.Add(busId);
            _bound.Remove(busId);
        }

        return Task.CompletedTask;
    }

    public Task<ExportStatus> GetStatusAsync(string busId, CancellationToken cancellationToken) =>
        Task.FromResult(_used ? ExportStatus.Used : ExportStatus.Available);

    public async Task ResetAsync(string busId, CancellationToken cancellationToken)
    {
        if (ResetGate is { } gate)
        {
            await Task.Run(() => gate.Wait(cancellationToken), CancellationToken.None);
        }

        _reset = true;
    }

    public async Task ExportAsync(UsbDevice device, Stream network, Func<CancellationToken, Task> exported, CancellationToken cancellationToken)
    {
        ResetBeforeExport = _reset;
        _reset = false;
        _used = true;
        try
        {
            await exported(cancellationToken);
            var buffer = new byte[4096];
            int read;
            while ((read = await network.ReadAsync(buffer, cancellationToken)) > 0)
            {
                await network.WriteAsync(buffer.AsMemory(0, read), cancellationToken);
            }
        }
        catch (Exception ex) when (ex is IOException or OperationCanceledException or ObjectDisposedException)
        {
            // Session ended.
        }
        finally
        {
            _used = false;
        }
    }

    public IReadOnlyList<VhciPort> Ports() => [];

    public int Attach(Socket socket, uint devId, UsbSpeed speed)
    {
        _attached[8] = socket;
        AttachedSocket.TrySetResult(socket);
        if (Interlocked.Increment(ref _attaches) > 1)
        {
            Reattached.TrySetResult(socket);
        }

        return 8;
    }

    public void Detach(int port)
    {
        DetachedPorts.Enqueue(port);
        if (_attached.TryRemove(port, out var socket))
        {
            socket.Dispose();
        }
    }
}
