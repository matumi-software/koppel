using System.Buffers.Binary;
using System.Text;

namespace Koppel.Core.Usb;

/// <summary>
/// The USB/IP operation messages (device list and import), version 1.1.1, as documented in the
/// Linux kernel's <c>Documentation/usb/usbip_protocol.rst</c>. All fields are big-endian. After a
/// successful import the same connection carries USB traffic, which the kernel handles itself.
/// </summary>
public static class UsbIpProtocol
{
    public const ushort Version = 0x0111;
    public const ushort OpReqDevList = 0x8005;
    public const ushort OpRepDevList = 0x0005;
    public const ushort OpReqImport = 0x8003;
    public const ushort OpRepImport = 0x0003;

    public const int HeaderSize = 8;
    public const int BusIdSize = 32;
    public const int DeviceSize = 312;
    public const int InterfaceSize = 4;

    /// <summary>Status values in a reply header. 0 is success.</summary>
    public const uint StatusOk = 0;
    public const uint StatusError = 1;

    public readonly record struct OpHeader(ushort Version, ushort Code, uint Status);

    public static async Task<OpHeader> ReadHeaderAsync(Stream stream, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(stream);
        var buffer = new byte[HeaderSize];
        await stream.ReadExactlyAsync(buffer, cancellationToken);
        return ParseHeader(buffer);
    }

    public static OpHeader ParseHeader(ReadOnlySpan<byte> source) => new(
        BinaryPrimitives.ReadUInt16BigEndian(source),
        BinaryPrimitives.ReadUInt16BigEndian(source[2..]),
        BinaryPrimitives.ReadUInt32BigEndian(source[4..]));

    /// <summary>Reads the body of an OP_REP_DEVLIST after its header: each device with its interfaces.</summary>
    public static IReadOnlyList<UsbDevice> ReadDevList(Stream stream)
    {
        ArgumentNullException.ThrowIfNull(stream);
        var count = new byte[4];
        stream.ReadExactly(count);
        var devices = new List<UsbDevice>();
        var buffer = new byte[DeviceSize];
        for (var n = BinaryPrimitives.ReadUInt32BigEndian(count); n > 0; n--)
        {
            stream.ReadExactly(buffer);
            var device = ReadDevice(buffer);
            var interfaces = new byte[buffer[311] * InterfaceSize];
            stream.ReadExactly(interfaces);
            devices.Add(device with
            {
                Interfaces = [.. interfaces.Chunk(InterfaceSize).Select(i => new UsbInterface(i[0], i[1], i[2]))],
            });
        }

        return devices;
    }

    /// <summary>The bus ID that follows an OP_REQ_IMPORT header.</summary>
    public static async Task<string> ReadBusIdAsync(Stream stream, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(stream);
        var buffer = new byte[BusIdSize];
        await stream.ReadExactlyAsync(buffer, cancellationToken);
        return ReadString(buffer);
    }

    public static byte[] DevListRequest() => Header(OpReqDevList, 0);

    public static byte[] ImportRequest(string busId)
    {
        var message = new byte[HeaderSize + BusIdSize];
        Header(OpReqImport, 0).CopyTo(message, 0);
        WriteString(message.AsSpan(HeaderSize, BusIdSize), busId);
        return message;
    }

    /// <summary>OP_REP_DEVLIST: the exportable devices, each followed by its interfaces.</summary>
    public static byte[] DevListReply(IReadOnlyCollection<UsbDevice> devices)
    {
        ArgumentNullException.ThrowIfNull(devices);
        var size = HeaderSize + 4 + devices.Sum(d => DeviceSize + (d.Interfaces.Count * InterfaceSize));
        var message = new byte[size];
        Header(OpRepDevList, StatusOk).CopyTo(message, 0);
        BinaryPrimitives.WriteUInt32BigEndian(message.AsSpan(HeaderSize), (uint)devices.Count);
        var offset = HeaderSize + 4;
        foreach (var device in devices)
        {
            WriteDevice(message.AsSpan(offset, DeviceSize), device);
            offset += DeviceSize;
            foreach (var iface in device.Interfaces)
            {
                message[offset] = iface.Class;
                message[offset + 1] = iface.SubClass;
                message[offset + 2] = iface.Protocol;
                offset += InterfaceSize;
            }
        }

        return message;
    }

    /// <summary>OP_REP_IMPORT. On success the device follows; on failure only the header.</summary>
    public static byte[] ImportReply(UsbDevice? device)
    {
        if (device is null)
        {
            return Header(OpRepImport, StatusError);
        }

        var message = new byte[HeaderSize + DeviceSize];
        Header(OpRepImport, StatusOk).CopyTo(message, 0);
        WriteDevice(message.AsSpan(HeaderSize), device);
        return message;
    }

    /// <summary>Reads the device that follows a successful OP_REP_IMPORT header.</summary>
    public static async Task<UsbDevice> ReadImportedDeviceAsync(Stream stream, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(stream);
        var buffer = new byte[DeviceSize];
        await stream.ReadExactlyAsync(buffer, cancellationToken);
        return ReadDevice(buffer);
    }

    public static byte[] Header(ushort code, uint status)
    {
        var header = new byte[HeaderSize];
        BinaryPrimitives.WriteUInt16BigEndian(header, Version);
        BinaryPrimitives.WriteUInt16BigEndian(header.AsSpan(2), code);
        BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(4), status);
        return header;
    }

    public static void WriteDevice(Span<byte> target, UsbDevice device)
    {
        ArgumentNullException.ThrowIfNull(device);
        if (target.Length < DeviceSize)
        {
            throw new ArgumentException($"Need {DeviceSize} bytes.", nameof(target));
        }

        target[..DeviceSize].Clear();
        WriteString(target[..256], device.Path);
        WriteString(target.Slice(256, BusIdSize), device.BusId);
        BinaryPrimitives.WriteUInt32BigEndian(target[288..], device.BusNum);
        BinaryPrimitives.WriteUInt32BigEndian(target[292..], device.DevNum);
        BinaryPrimitives.WriteUInt32BigEndian(target[296..], (uint)device.Speed);
        BinaryPrimitives.WriteUInt16BigEndian(target[300..], device.VendorId);
        BinaryPrimitives.WriteUInt16BigEndian(target[302..], device.ProductId);
        BinaryPrimitives.WriteUInt16BigEndian(target[304..], device.BcdDevice);
        target[306] = device.Class;
        target[307] = device.SubClass;
        target[308] = device.Protocol;
        target[309] = device.ConfigurationValue;
        target[310] = device.NumConfigurations;
        target[311] = (byte)device.Interfaces.Count;
    }

    public static UsbDevice ReadDevice(ReadOnlySpan<byte> source) => new(
        BusId: ReadString(source.Slice(256, BusIdSize)),
        Path: ReadString(source[..256]),
        BusNum: BinaryPrimitives.ReadUInt32BigEndian(source[288..]),
        DevNum: BinaryPrimitives.ReadUInt32BigEndian(source[292..]),
        Speed: (UsbSpeed)BinaryPrimitives.ReadUInt32BigEndian(source[296..]),
        VendorId: BinaryPrimitives.ReadUInt16BigEndian(source[300..]),
        ProductId: BinaryPrimitives.ReadUInt16BigEndian(source[302..]),
        BcdDevice: BinaryPrimitives.ReadUInt16BigEndian(source[304..]),
        Class: source[306],
        SubClass: source[307],
        Protocol: source[308],
        ConfigurationValue: source[309],
        NumConfigurations: source[310],
        Interfaces: [],
        Manufacturer: null,
        Product: null,
        Serial: null,
        Driver: null);

    private static void WriteString(Span<byte> target, string value)
    {
        var bytes = Encoding.ASCII.GetBytes(value);
        // Always leave room for the terminating zero.
        bytes.AsSpan(0, Math.Min(bytes.Length, target.Length - 1)).CopyTo(target);
    }

    private static string ReadString(ReadOnlySpan<byte> source)
    {
        var end = source.IndexOf((byte)0);
        return Encoding.ASCII.GetString(end < 0 ? source : source[..end]);
    }
}
