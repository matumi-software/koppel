using System.Globalization;
using System.Net.Sockets;

namespace Koppel.Core.Usb.Linux;

/// <summary>
/// The Linux kernel's USB/IP client side (<c>vhci-hcd</c>), driven through sysfs, the way
/// <c>usbip attach</c> does it. Attached devices appear on the host kernel. Needs root and the
/// <c>vhci-hcd</c> module.
/// </summary>
/// <param name="sysRoot">Normally <c>/sys</c>; tests point it at a fake tree.</param>
public sealed class SysfsVhci(string sysRoot = "/sys") : IUsbAttacher
{
    private readonly object _attachLock = new();

    /// <summary>
    /// The controller's attribute folder. Older kernels use the platform device; a 2026 change moved
    /// attach, detach and status under the driver, so both are tried.
    /// </summary>
    private string? Controller => new[]
    {
        Path.Combine(sysRoot, "devices", "platform", "vhci_hcd.0"),
        Path.Combine(sysRoot, "bus", "platform", "drivers", "vhci_hcd"),
    }.FirstOrDefault(dir => File.Exists(Path.Combine(dir, "attach")));

    public string? Unavailable => Controller is null
        ? "The vhci-hcd kernel module is not loaded. Run 'modprobe vhci-hcd' on the host (add it to /etc/modules to keep it)."
        : null;

    /// <summary>Parses the status files: <c>hub port sta spd dev sockfd local_busid</c> per line.</summary>
    public IReadOnlyList<VhciPort> Ports()
    {
        if (Controller is not { } controller)
        {
            return [];
        }

        var ports = new List<VhciPort>();
        foreach (var file in Directory.EnumerateFiles(controller, "status*").Order(StringComparer.Ordinal))
        {
            foreach (var line in File.ReadAllLines(file).Skip(1))
            {
                var parts = line.Split(' ', StringSplitOptions.RemoveEmptyEntries);
                if (parts.Length >= 7
                    && int.TryParse(parts[1], NumberStyles.Integer, CultureInfo.InvariantCulture, out var port)
                    && int.TryParse(parts[2], NumberStyles.Integer, CultureInfo.InvariantCulture, out var status)
                    && uint.TryParse(parts[4], NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var devId))
                {
                    ports.Add(new VhciPort(port, parts[0], status, devId, parts[6] == "0-0" ? null : parts[6]));
                }
            }
        }

        return ports;
    }

    public int Attach(Socket socket, uint devId, UsbSpeed speed)
    {
        var controller = Controller ?? throw new UsbHostException(Unavailable!);
        // SuperSpeed devices need a port on the SuperSpeed hub; everything else uses the high-speed hub.
        var hub = speed >= UsbSpeed.Super ? "ss" : "hs";
        var fd = UsbSockets.FileDescriptor(socket);

        // Picking a free port and writing the attach line is one step: two attaches at once would
        // otherwise pick the same port. If the kernel still refuses (someone outside Koppel took the
        // port in between), the next free port gets one try.
        lock (_attachLock)
        {
            var tried = new List<int>();
            for (var attempt = 0; ; attempt++)
            {
                var port = Ports().FirstOrDefault(p => p.IsFree && p.Hub == hub && !tried.Contains(p.Port))
                    ?? throw new UsbHostException("All virtual USB ports are in use. Detach a device first.");
                var line = string.Create(CultureInfo.InvariantCulture, $"{port.Port} {fd} {devId} {(int)speed}");
                try
                {
                    File.WriteAllText(Path.Combine(controller, "attach"), line);
                    return port.Port;
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
                {
                    if (attempt >= 1 || ex is UnauthorizedAccessException)
                    {
                        throw new UsbHostException($"The kernel refused to attach the device: {ex.Message}", ex);
                    }

                    tried.Add(port.Port);
                }
            }
        }
    }

    public void Detach(int port)
    {
        var controller = Controller ?? throw new UsbHostException(Unavailable!);
        try
        {
            File.WriteAllText(Path.Combine(controller, "detach"), port.ToString(CultureInfo.InvariantCulture));
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            throw new UsbHostException($"The kernel refused to detach port {port}: {ex.Message}", ex);
        }
    }
}
