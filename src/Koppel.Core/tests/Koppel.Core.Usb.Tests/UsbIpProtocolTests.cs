using System.Buffers.Binary;

namespace Koppel.Core.Usb.Tests;

public sealed class UsbIpProtocolTests
{
    internal static readonly UsbDevice Ftdi = new(
        BusId: "1-1.4",
        Path: "/sys/devices/pci0000:00/0000:00:14.0/usb1/1-1/1-1.4",
        BusNum: 1,
        DevNum: 5,
        Speed: UsbSpeed.Full,
        VendorId: 0x0403,
        ProductId: 0x6001,
        BcdDevice: 0x0600,
        Class: 0,
        SubClass: 0,
        Protocol: 0,
        ConfigurationValue: 1,
        NumConfigurations: 1,
        Interfaces: [new UsbInterface(0xff, 0xff, 0xff)],
        Manufacturer: "FTDI",
        Product: "FT232R USB UART",
        Serial: "A50285BI",
        Driver: "usbip-host");

    [Fact]
    public void Import_request_is_header_plus_a_32_byte_bus_id()
    {
        var request = UsbIpProtocol.ImportRequest("1-1.4");

        request.Should().HaveCount(40);
        BinaryPrimitives.ReadUInt16BigEndian(request).Should().Be(0x0111);
        BinaryPrimitives.ReadUInt16BigEndian(request.AsSpan(2)).Should().Be(0x8003);
        System.Text.Encoding.ASCII.GetString(request, 8, 5).Should().Be("1-1.4");
        request[13].Should().Be(0);
    }

    [Fact]
    public async Task Import_reply_round_trips_the_device_fields()
    {
        var reply = UsbIpProtocol.ImportReply(Ftdi);
        reply.Should().HaveCount(8 + 312);
        using var stream = new MemoryStream(reply);

        var header = await UsbIpProtocol.ReadHeaderAsync(stream, TestContext.Current.CancellationToken);
        var device = await UsbIpProtocol.ReadImportedDeviceAsync(stream, TestContext.Current.CancellationToken);

        header.Should().Be(new UsbIpProtocol.OpHeader(0x0111, 0x0003, 0));
        device.BusId.Should().Be("1-1.4");
        device.Path.Should().Be(Ftdi.Path);
        device.DevId.Should().Be((1u << 16) | 5);
        device.Speed.Should().Be(UsbSpeed.Full);
        device.VidPid.Should().Be("0403:6001");
        device.ConfigurationValue.Should().Be(1);
    }

    [Fact]
    public void Devlist_reply_counts_devices_and_carries_their_interfaces()
    {
        var reply = UsbIpProtocol.DevListReply([Ftdi]);

        reply.Should().HaveCount(8 + 4 + 312 + 4);
        BinaryPrimitives.ReadUInt32BigEndian(reply.AsSpan(8)).Should().Be(1);
        reply[8 + 4 + 311].Should().Be(1, "bNumInterfaces");
        reply[8 + 4 + 312].Should().Be(0xff, "bInterfaceClass");
    }

    [Fact]
    public void Failed_import_reply_is_only_a_header_with_an_error_status()
    {
        var reply = UsbIpProtocol.ImportReply(null);

        reply.Should().HaveCount(8);
        BinaryPrimitives.ReadUInt32BigEndian(reply.AsSpan(4)).Should().Be(1);
    }
}
