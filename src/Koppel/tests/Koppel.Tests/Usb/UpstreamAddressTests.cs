using Koppel.Usb;

namespace Koppel.Tests.Usb;

public sealed class UpstreamAddressTests
{
    [Theory]
    [InlineData("usbipd.lab", "usbipd.lab", 3240)]
    [InlineData("usbipd.lab:3241", "usbipd.lab", 3241)]
    [InlineData("192.168.1.20:4000", "192.168.1.20", 4000)]
    [InlineData("[::1]:3241", "::1", 3241)]
    [InlineData("[fe80::1]", "fe80::1", 3240)]
    [InlineData("fe80::1", "fe80::1", 3240)]
    [InlineData("  host:65535 ", "host", 65535)]
    public void Parse_reads_host_and_port(string value, string host, int port) =>
        UpstreamAddress.Parse(value).Should().Be(new UpstreamAddress(host, port));

    [Theory]
    [InlineData("host:")]
    [InlineData("host:abc")]
    [InlineData("host:0")]
    [InlineData("host:65536")]
    [InlineData(":3240")]
    [InlineData("[::1")]
    [InlineData("[host]:3240")]
    [InlineData("[::1]3240")]
    [InlineData("::1:3240x")]
    [InlineData("")]
    public void Parse_rejects_a_bad_value_and_names_the_setting(string value)
    {
        var parse = () => UpstreamAddress.Parse(value);

        parse.Should().Throw<InvalidOperationException>().Which.Message.Should().Contain("Usb:Upstream");
    }
}
