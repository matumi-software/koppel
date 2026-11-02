using System.Net.Sockets;
using Koppel.Core.Usb.Linux;

namespace Koppel.Core.Usb.Tests;

/// <summary>Drives the sysfs adapters against a fake /sys tree. The real kernel is not involved.</summary>
public sealed class SysfsTests : IDisposable
{
    private readonly string _sys = Path.Combine(Path.GetTempPath(), "koppel-sysfs", Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(_sys))
        {
            Directory.Delete(_sys, recursive: true);
        }
    }

    [Fact]
    public async Task Lists_devices_and_skips_hubs_root_hubs_and_interfaces()
    {
        AddDevice("1-1.4", "0403", "6001", "00", "12", "ftdi_sio", product: "FT232R USB UART");
        AddDevice("1-1", "05e3", "0610", "09", "480", "hub");
        Directory.CreateDirectory(Path.Combine(Devices, "usb1"));
        var host = new SysfsUsbHost(_sys);

        var devices = await host.ListDevicesAsync(TestContext.Current.CancellationToken);

        devices.Should().ContainSingle();
        var device = devices[0];
        device.BusId.Should().Be("1-1.4");
        device.VidPid.Should().Be("0403:6001");
        device.Speed.Should().Be(UsbSpeed.Full);
        device.Product.Should().Be("FT232R USB UART");
        device.Driver.Should().Be("ftdi_sio");
        if (!OperatingSystem.IsWindows())
        {
            device.Interfaces.Should().ContainSingle().Which.Should().Be(new UsbInterface(0xff, 0xff, 0xff));
        }
    }

    [Fact]
    public async Task Devices_attached_from_another_server_are_not_listed()
    {
        Assert.SkipWhen(OperatingSystem.IsWindows(), "Needs real symlinks, as sysfs has; runs in the Linux container.");
        var real = Directory.CreateDirectory(Path.Combine(_sys, "devices", "platform", "vhci_hcd.0", "usb1", "1-1")).FullName;
        Directory.CreateDirectory(Devices);
        Directory.CreateSymbolicLink(Path.Combine(Devices, "1-1"), real);
        File.WriteAllText(Path.Combine(real, "idVendor"), "046d");
        File.WriteAllText(Path.Combine(real, "idProduct"), "c548");
        File.WriteAllText(Path.Combine(real, "bDeviceClass"), "00");
        AddDevice("3-2", "0403", "6001", "00", "12", "ftdi_sio");

        var devices = await new SysfsUsbHost(_sys).ListDevicesAsync(TestContext.Current.CancellationToken);

        devices.Select(d => d.BusId).Should().Equal("3-2");
    }

    [Fact]
    public async Task Share_claims_the_bus_id_unbinds_the_old_driver_and_binds_usbip_host()
    {
        AddDevice("1-1.4", "0403", "6001", "00", "12", "ftdi_sio");
        var usbipHost = Directory.CreateDirectory(Path.Combine(_sys, "bus", "usb", "drivers", "usbip-host")).FullName;
        var host = new SysfsUsbHost(_sys);

        await host.ShareAsync("1-1.4", TestContext.Current.CancellationToken);

        File.ReadAllText(Path.Combine(usbipHost, "match_busid")).Should().Be("add 1-1.4");
        File.ReadAllText(Path.Combine(Devices, "1-1.4", "driver", "unbind")).Should().Be("1-1.4");
        File.ReadAllText(Path.Combine(usbipHost, "bind")).Should().Be("1-1.4");
    }

    [Fact]
    public async Task Share_without_the_module_explains_what_to_load()
    {
        AddDevice("1-1.4", "0403", "6001", "00", "12", "ftdi_sio");
        var host = new SysfsUsbHost(_sys);

        var share = () => host.ShareAsync("1-1.4", TestContext.Current.CancellationToken);

        await share.Should().ThrowAsync<UsbHostException>().WithMessage("*modprobe usbip-host*");
    }

    [Fact]
    public async Task Bus_ids_that_could_escape_sysfs_are_refused()
    {
        Directory.CreateDirectory(Path.Combine(_sys, "bus", "usb", "drivers", "usbip-host"));
        var host = new SysfsUsbHost(_sys);

        var share = () => host.ShareAsync("../../etc", TestContext.Current.CancellationToken);

        await share.Should().ThrowAsync<UsbHostException>();
    }

    [Fact]
    public async Task Reset_goes_through_the_usbfs_node_for_the_device_bus_and_number()
    {
        AddDevice("1-1.4", "046d", "c52b", "00", "12", "usbip-host");
        var dev = Path.Combine(_sys, "dev");
        var host = new SysfsUsbHost(_sys, dev);

        var reset = () => host.ResetAsync("1-1.4", TestContext.Current.CancellationToken);

        await reset.Should().ThrowAsync<UsbHostException>().WithMessage($"*{Path.Combine(dev, "bus", "usb", "001", "005")}*");
    }

    [Fact]
    public async Task Reset_reports_a_node_the_kernel_will_not_reset()
    {
        Assert.SkipWhen(OperatingSystem.IsWindows(), "Calls the Linux ioctl; runs in the Linux container.");
        AddDevice("1-1.4", "046d", "c52b", "00", "12", "usbip-host");
        var node = Directory.CreateDirectory(Path.Combine(_sys, "dev", "bus", "usb", "001")).FullName;
        File.WriteAllText(Path.Combine(node, "005"), "");
        var host = new SysfsUsbHost(_sys, Path.Combine(_sys, "dev"));

        var reset = () => host.ResetAsync("1-1.4", TestContext.Current.CancellationToken);

        await reset.Should().ThrowAsync<UsbHostException>().WithMessage("*refused to reset*");
    }

    [Fact]
    public async Task A_bus_id_with_a_trailing_newline_is_refused()
    {
        Directory.CreateDirectory(Path.Combine(_sys, "bus", "usb", "drivers", "usbip-host"));
        var host = new SysfsUsbHost(_sys);

        var share = () => host.ShareAsync("1-1\n", TestContext.Current.CancellationToken);

        await share.Should().ThrowAsync<UsbHostException>();
    }

    [Fact]
    public async Task Export_hands_the_kernel_a_socket_and_relays_the_network_stream_through_the_other_end()
    {
        var ct = TestContext.Current.CancellationToken;
        AddDevice("1-1.4", "0403", "6001", "00", "12", "usbip-host");
        File.WriteAllText(Path.Combine(Devices, "1-1.4", "usbip_status"), "1");
        var host = new SysfsUsbHost(_sys);
        var device = (await host.ListDevicesAsync(ct)).Single();
        var (client, network) = await UsbSockets.CreateLoopbackPairAsync(ct);
        using var clientSocket = client;
        await using var networkStream = new NetworkStream(network, ownsSocket: true);
        var exported = false;

        using var cancel = CancellationTokenSource.CreateLinkedTokenSource(ct);
        var session = host.ExportAsync(device, networkStream, _ =>
        {
            exported = true;
            return Task.CompletedTask;
        }, cancel.Token);
        await Task.Delay(300, ct);
        await cancel.CancelAsync();
        await session.WaitAsync(TimeSpan.FromSeconds(5), ct);

        exported.Should().BeTrue();
        File.ReadAllText(Path.Combine(Devices, "1-1.4", "usbip_sockfd")).Should().MatchRegex(@"^\d+$");
    }

    [Fact]
    public async Task Export_refuses_a_device_that_is_in_use()
    {
        AddDevice("1-1.4", "0403", "6001", "00", "12", "usbip-host");
        File.WriteAllText(Path.Combine(Devices, "1-1.4", "usbip_status"), "2");
        var host = new SysfsUsbHost(_sys);
        var device = (await host.ListDevicesAsync(TestContext.Current.CancellationToken)).Single();

        var export = () => host.ExportAsync(device, new MemoryStream(), _ => Task.CompletedTask, TestContext.Current.CancellationToken);

        await export.Should().ThrowAsync<UsbHostException>().WithMessage("*not available*");
    }

    [Fact]
    public void Vhci_picks_a_free_port_on_the_right_hub_and_writes_the_attach_line()
    {
        var vhci = Directory.CreateDirectory(Path.Combine(_sys, "devices", "platform", "vhci_hcd.0")).FullName;
        File.WriteAllText(Path.Combine(vhci, "attach"), "");
        File.WriteAllText(Path.Combine(vhci, "status"), string.Join('\n',
            "hub port sta spd dev      sockfd local_busid",
            "hs  0000 006 002 00010005 000003 1-1",
            "hs  0001 004 000 00000000 000000 0-0",
            "ss  0008 004 000 00000000 000000 0-0"));
        var attacher = new SysfsVhci(_sys);
        using var socket = new System.Net.Sockets.Socket(System.Net.Sockets.SocketType.Stream, System.Net.Sockets.ProtocolType.Tcp);

        attacher.Ports().Should().HaveCount(3);
        attacher.Ports()[0].LocalBusId.Should().Be("1-1");
        var port = attacher.Attach(socket, devId: 65541, UsbSpeed.Full);

        port.Should().Be(1);
        File.ReadAllText(Path.Combine(vhci, "attach")).Should().MatchRegex(@"^1 \d+ 65541 2$");
    }

    private string Devices => Path.Combine(_sys, "bus", "usb", "devices");

    private void AddDevice(string busId, string vid, string pid, string deviceClass, string speed, string driver, string? product = null)
    {
        var dir = Directory.CreateDirectory(Path.Combine(Devices, busId)).FullName;
        void Write(string name, string value) => File.WriteAllText(Path.Combine(dir, name), value + "\n");
        Write("idVendor", vid);
        Write("idProduct", pid);
        Write("bcdDevice", "0600");
        Write("bDeviceClass", deviceClass);
        Write("bDeviceSubClass", "00");
        Write("bDeviceProtocol", "00");
        Write("bConfigurationValue", "1");
        Write("bNumConfigurations", "1");
        Write("busnum", "1");
        Write("devnum", "5");
        Write("speed", speed);
        if (product is not null)
        {
            Write("product", product);
        }

        // A real driver entry is a symlink; the fake tree records the driver name in a file instead.
        var driverDir = Directory.CreateDirectory(Path.Combine(dir, "driver")).FullName;
        File.WriteAllText(Path.Combine(driverDir, "name"), driver);
        // Interface folders are named like 1-1.4:1.0, which Windows cannot create. Run these tests on
        // Linux (see CLAUDE.md) to cover interfaces.
        if (OperatingSystem.IsWindows())
        {
            return;
        }

        var iface = Directory.CreateDirectory(Path.Combine(dir, $"{busId}:1.0")).FullName;
        File.WriteAllText(Path.Combine(iface, "bInterfaceClass"), "ff");
        File.WriteAllText(Path.Combine(iface, "bInterfaceSubClass"), "ff");
        File.WriteAllText(Path.Combine(iface, "bInterfaceProtocol"), "ff");
    }
}
