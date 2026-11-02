using System.Globalization;
using System.Net.Sockets;

namespace Koppel.Core.Usb.Linux;

/// <summary>
/// The Linux kernel's USB/IP server side (<c>usbip-host</c>), driven through sysfs, the way the
/// <c>usbip bind</c> and <c>usbipd</c> tools do it. Needs root and the <c>usbip-host</c> module.
/// </summary>
/// <param name="sysRoot">Normally <c>/sys</c>; tests point it at a fake tree.</param>
/// <param name="devRoot">Normally <c>/dev</c>, where the usbfs device nodes are.</param>
public sealed class SysfsUsbHost(string sysRoot = "/sys", string devRoot = "/dev") : IUsbHost
{
    private string Devices => Path.Combine(sysRoot, "bus", "usb", "devices");

    private string UsbipHost => Path.Combine(sysRoot, "bus", "usb", "drivers", "usbip-host");

    public string? Unavailable => Directory.Exists(UsbipHost)
        ? null
        : "The usbip-host kernel module is not loaded. Run 'modprobe usbip-host' on the host (add it to /etc/modules to keep it).";

    public bool CanShare => true;

    public bool CanUnshare => true;

    public bool CanReset => true;

    public bool IsShared(UsbDevice device) => device?.Driver == "usbip-host";

    public string SharingAdvice(string busId, bool share) => Unavailable ?? "Koppel shares devices on this machine itself.";

    // The sysfs reads and writes are small and local, but they are still file I/O: they run off the caller's thread.
    public Task<IReadOnlyList<UsbDevice>> ListDevicesAsync(CancellationToken cancellationToken) =>
        Task.Run(() => (IReadOnlyList<UsbDevice>)ListDevices(), cancellationToken);

    public Task ShareAsync(string busId, CancellationToken cancellationToken) => Task.Run(() => Share(busId), cancellationToken);

    public Task UnshareAsync(string busId, CancellationToken cancellationToken) => Task.Run(() => Unshare(busId), cancellationToken);

    public Task<ExportStatus> GetStatusAsync(string busId, CancellationToken cancellationToken) =>
        Task.Run(() => GetStatus(busId), cancellationToken);

    public Task ResetAsync(string busId, CancellationToken cancellationToken) => Task.Run(() => Reset(busId), cancellationToken);

    /// <summary>
    /// Hands the kernel one end of a loopback socket pair, which from then on carries the device's USB
    /// traffic, and relays the other end through <paramref name="network"/>.
    /// </summary>
    public async Task ExportAsync(UsbDevice device, Stream network, Func<CancellationToken, Task> exported, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(device);
        ArgumentNullException.ThrowIfNull(network);
        ArgumentNullException.ThrowIfNull(exported);
        var busId = ValidBusId(device.BusId);
        if (GetStatus(busId) != ExportStatus.Available)
        {
            throw new UsbHostException($"Device {busId} is not available to export. It may be in use by another client.");
        }

        var (kernelEnd, relayEnd) = await UsbSockets.CreateLoopbackPairAsync(cancellationToken);
        using (kernelEnd)
        await using (var relay = new NetworkStream(relayEnd, ownsSocket: true))
        {
            Write(Path.Combine(Devices, busId, "usbip_sockfd"), UsbSockets.FileDescriptor(kernelEnd).ToString(CultureInfo.InvariantCulture));
            await exported(cancellationToken);
            // Runs until the client detaches or the link drops; the kernel then frees the device.
            await UsbSockets.RelayAsync(network, relay, cancellationToken);
        }
    }

    private List<UsbDevice> ListDevices()
    {
        if (!Directory.Exists(Devices))
        {
            return [];
        }

        return [.. Directory.EnumerateDirectories(Devices)
            .Select(Path.GetFileName)
            .Where(name => name is not null && UsbBusId.IsValid(name) && !IsImported(name))
            .Select(name => ReadDevice(name!))
            .Where(device => device is not null && !device.IsHub)
            .Cast<UsbDevice>()
            .OrderBy(device => device.BusId, StringComparer.Ordinal)];
    }

    private void Share(string busId)
    {
        RequireModule();
        var device = Path.Combine(Devices, ValidBusId(busId));
        Write(Path.Combine(UsbipHost, "match_busid"), $"add {busId}");
        try
        {
            if (DriverOf(device) is { } driver && driver != "usbip-host")
            {
                Write(Path.Combine(device, "driver", "unbind"), busId);
            }

            if (DriverOf(device) != "usbip-host")
            {
                Write(Path.Combine(UsbipHost, "bind"), busId);
            }
        }
        catch (UsbHostException)
        {
            // A share that did not take must not leave the bus ID on the match list, or the device binds by itself later.
            try
            {
                Write(Path.Combine(UsbipHost, "match_busid"), $"del {busId}");
            }
            catch (UsbHostException)
            {
                // The first failure is the one to report.
            }

            throw;
        }
    }

    private void Unshare(string busId)
    {
        RequireModule();
        var device = Path.Combine(Devices, ValidBusId(busId));
        if (DriverOf(device) == "usbip-host")
        {
            Write(Path.Combine(UsbipHost, "unbind"), busId);
        }

        Write(Path.Combine(UsbipHost, "match_busid"), $"del {busId}");
        // Let the kernel find the normal driver again.
        Write(Path.Combine(sysRoot, "bus", "usb", "drivers_probe"), busId);
    }

    private ExportStatus GetStatus(string busId)
    {
        var status = ReadAttribute(Path.Combine(Devices, ValidBusId(busId)), "usbip_status");
        return int.TryParse(status, NumberStyles.Integer, CultureInfo.InvariantCulture, out var value) && Enum.IsDefined((ExportStatus)value)
            ? (ExportStatus)value
            : ExportStatus.NotExported;
    }

    private void Reset(string busId)
    {
        var device = Path.Combine(Devices, ValidBusId(busId));
        var node = Path.Combine(
            devRoot,
            "bus",
            "usb",
            ReadUInt(device, "busnum").ToString("D3", CultureInfo.InvariantCulture),
            ReadUInt(device, "devnum").ToString("D3", CultureInfo.InvariantCulture));
        if (!File.Exists(node))
        {
            throw new UsbHostException($"Cannot reset {busId}: there is no device node {node}.");
        }

        UsbDeviceFs.Reset(node);
    }

    /// <summary>
    /// Devices attached from another server sit under vhci-hcd. They are on loan to this machine, so they
    /// are never offered for sharing here.
    /// </summary>
    private bool IsImported(string busId) =>
        new DirectoryInfo(Path.Combine(Devices, busId)).LinkTarget?.Contains("vhci_hcd", StringComparison.Ordinal) == true;

    private UsbDevice? ReadDevice(string busId)
    {
        var path = Path.Combine(Devices, busId);
        var vendor = ReadHex16(path, "idVendor");
        var product = ReadHex16(path, "idProduct");
        if (vendor is null || product is null)
        {
            return null;
        }

        var interfaces = Directory.EnumerateDirectories(path, $"{busId}:*")
            .OrderBy(p => p, StringComparer.Ordinal)
            .Select(i => new UsbInterface(ReadHex8(i, "bInterfaceClass"), ReadHex8(i, "bInterfaceSubClass"), ReadHex8(i, "bInterfaceProtocol")))
            .ToList();

        return new UsbDevice(
            BusId: busId,
            Path: path,
            BusNum: ReadUInt(path, "busnum"),
            DevNum: ReadUInt(path, "devnum"),
            Speed: ParseSpeed(ReadAttribute(path, "speed")),
            VendorId: vendor.Value,
            ProductId: product.Value,
            BcdDevice: ReadHex16(path, "bcdDevice") ?? 0,
            Class: ReadHex8(path, "bDeviceClass"),
            SubClass: ReadHex8(path, "bDeviceSubClass"),
            Protocol: ReadHex8(path, "bDeviceProtocol"),
            ConfigurationValue: (byte)ReadUInt(path, "bConfigurationValue"),
            NumConfigurations: (byte)ReadUInt(path, "bNumConfigurations"),
            Interfaces: interfaces,
            Manufacturer: ReadAttribute(path, "manufacturer"),
            Product: ReadAttribute(path, "product"),
            Serial: ReadAttribute(path, "serial"),
            Driver: DriverOf(path));
    }

    /// <summary>Speeds as sysfs prints them, in Mbit/s.</summary>
    private static UsbSpeed ParseSpeed(string? speed) => speed switch
    {
        "1.5" => UsbSpeed.Low,
        "12" => UsbSpeed.Full,
        "480" => UsbSpeed.High,
        "53.3-480" => UsbSpeed.Wireless,
        "5000" => UsbSpeed.Super,
        "10000" or "20000" => UsbSpeed.SuperPlus,
        _ => UsbSpeed.Unknown,
    };

    private static string? DriverOf(string devicePath)
    {
        var link = new DirectoryInfo(Path.Combine(devicePath, "driver"));
        if (!link.Exists)
        {
            return null;
        }

        // A real sysfs driver entry is a symlink to /sys/bus/usb/drivers/<name>; a fake tree may use a folder.
        return link.LinkTarget is { } target ? Path.GetFileName(target.TrimEnd('/')) : ReadAttribute(link.FullName, "name");
    }

    private static string? ReadAttribute(string directory, string name)
    {
        var file = Path.Combine(directory, name);
        try
        {
            return File.Exists(file) ? File.ReadAllText(file).Trim() : null;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    private static uint ReadUInt(string directory, string name) =>
        uint.TryParse(ReadAttribute(directory, name), NumberStyles.Integer, CultureInfo.InvariantCulture, out var value) ? value : 0;

    private static ushort? ReadHex16(string directory, string name) =>
        ushort.TryParse(ReadAttribute(directory, name), NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var value) ? value : null;

    private static byte ReadHex8(string directory, string name) =>
        byte.TryParse(ReadAttribute(directory, name), NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var value) ? value : (byte)0;

    private void RequireModule()
    {
        if (Unavailable is { } reason)
        {
            throw new UsbHostException(reason);
        }
    }

    private static string ValidBusId(string busId) => UsbBusId.Require(busId);

    private static void Write(string file, string value)
    {
        try
        {
            File.WriteAllText(file, value);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            throw new UsbHostException($"The kernel refused '{value}' on {file}: {ex.Message}", ex);
        }
    }
}
