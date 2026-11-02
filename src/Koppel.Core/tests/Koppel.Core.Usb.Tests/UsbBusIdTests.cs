using System.Net.Sockets;
using Koppel.Core.Usb.Windows;

namespace Koppel.Core.Usb.Tests;

public sealed class UsbBusIdTests
{
    [Theory]
    [InlineData("1-1")]
    [InlineData("1-1.4")]
    [InlineData("3-2.1.7.4")]
    [InlineData("12-10")]
    public void Valid_bus_ids_are_accepted(string busId) => UsbBusId.IsValid(busId).Should().BeTrue();

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("1")]
    [InlineData("1-")]
    [InlineData("-1")]
    [InlineData("1-1.")]
    [InlineData("1-1.4 --stop")]
    [InlineData("--bus-id=1-1")]
    [InlineData("1-1.4\n")]
    [InlineData("a-b")]
    [InlineData("1-1.4;calc")]
    [InlineData("١-١")]
    public void Anything_else_is_refused(string? busId) => UsbBusId.IsValid(busId).Should().BeFalse();

    [Fact]
    public void A_bus_id_longer_than_the_kernel_allows_is_refused()
    {
        var longest = "1-" + string.Join('.', Enumerable.Repeat("1", 15));
        longest.Length.Should().BeLessThanOrEqualTo(UsbBusId.MaxLength);
        UsbBusId.IsValid(longest).Should().BeTrue();
        UsbBusId.IsValid(longest + ".1").Should().BeFalse();
    }

    [Fact]
    public async Task Driver_refuses_a_bus_id_before_it_starts_usbip()
    {
        var driver = new UsbipWin2Driver("does-not-exist.exe");

        var attach = () => driver.AttachAsync("127.0.0.1", 3242, "1-1 --stop", CancellationToken.None);
        var detach = () => driver.DetachAsync("127.0.0.1", 3242, "--all", 1, CancellationToken.None);

        (await attach.Should().ThrowAsync<UsbHostException>()).Which.Message.Should().Contain("not a USB bus ID");
        (await detach.Should().ThrowAsync<UsbHostException>()).Which.Message.Should().Contain("not a USB bus ID");
    }

    [Fact]
    public void Keepalive_is_switched_on_for_the_socket()
    {
        using var socket = new Socket(SocketType.Stream, ProtocolType.Tcp);

        UsbSockets.EnableKeepAlive(socket);

        Convert.ToInt32(socket.GetSocketOption(SocketOptionLevel.Socket, SocketOptionName.KeepAlive), System.Globalization.CultureInfo.InvariantCulture).Should().NotBe(0);
        Convert.ToInt32(socket.GetSocketOption(SocketOptionLevel.Tcp, SocketOptionName.TcpKeepAliveTime), System.Globalization.CultureInfo.InvariantCulture).Should().Be(30);
        Convert.ToInt32(socket.GetSocketOption(SocketOptionLevel.Tcp, SocketOptionName.TcpKeepAliveInterval), System.Globalization.CultureInfo.InvariantCulture).Should().Be(10);
    }
}
