using System.Globalization;

namespace Koppel.Core.Usb;

/// <summary>Device speed, numbered as the kernel's <c>enum usb_device_speed</c> and the USB/IP wire format.</summary>
public enum UsbSpeed
{
    Unknown = 0,
    Low = 1,
    Full = 2,
    High = 3,
    Wireless = 4,
    Super = 5,
    SuperPlus = 6,
}

public sealed record UsbInterface(byte Class, byte SubClass, byte Protocol);

/// <summary>A USB device as the host sees it, with what USB/IP needs to describe it.</summary>
/// <param name="BusId">The kernel's bus ID, such as <c>1-1.4</c>. Stable for a port, not for a device.</param>
/// <param name="Driver">The driver bound to it, such as <c>ftdi_sio</c> or <c>usbip-host</c>; null when none.</param>
public sealed record UsbDevice(
    string BusId,
    string Path,
    uint BusNum,
    uint DevNum,
    UsbSpeed Speed,
    ushort VendorId,
    ushort ProductId,
    ushort BcdDevice,
    byte Class,
    byte SubClass,
    byte Protocol,
    byte ConfigurationValue,
    byte NumConfigurations,
    IReadOnlyList<UsbInterface> Interfaces,
    string? Manufacturer,
    string? Product,
    string? Serial,
    string? Driver)
{
    /// <summary>USB/IP's device ID: bus number in the high 16 bits, device number in the low.</summary>
    public uint DevId => (BusNum << 16) | DevNum;

    /// <summary><c>0403:6001</c>.</summary>
    public string VidPid => string.Create(CultureInfo.InvariantCulture, $"{VendorId:x4}:{ProductId:x4}");

    /// <summary>Hubs are never shared: sharing one would take every device behind it.</summary>
    public bool IsHub => Class == 0x09;
}

/// <summary>A USB operation the host refused. The message is written for the UI.</summary>
public sealed class UsbHostException(string message, Exception? inner = null) : Exception(message, inner);
