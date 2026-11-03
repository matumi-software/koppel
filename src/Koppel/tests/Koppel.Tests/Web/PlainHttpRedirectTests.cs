using System.Buffers;
using System.Net;
using System.Net.Sockets;
using System.Text;
using Koppel.Web;

namespace Koppel.Tests.Web;

public sealed class PlainHttpRedirectTests
{
    private static readonly IPEndPoint Local = new(IPAddress.Parse("192.168.1.20"), 5240);

    private static string? Location(string head) => PlainHttpRedirect.Location(new ReadOnlySequence<byte>(Encoding.ASCII.GetBytes(head)), Local);

    [Fact]
    public void Location_keeps_the_host_port_and_path() =>
        Location("GET /devices?tab=shared HTTP/1.1\r\nHost: lab-rack-3:5240\r\nAccept: */*\r\n\r\n")
            .Should().Be("https://lab-rack-3:5240/devices?tab=shared");

    [Fact]
    public void Location_uses_the_connection_address_when_the_host_header_is_missing_or_odd()
    {
        Location("GET / HTTP/1.0\r\n\r\n").Should().Be("https://192.168.1.20:5240/");
        Location("GET / HTTP/1.1\r\nHost: evil.example/x?\r\n\r\n").Should().Be("https://192.168.1.20:5240/");
        Location("GET / HTTP/1.1\r\nHost: user@evil.example\r\n\r\n").Should().Be("https://192.168.1.20:5240/");
    }

    [Fact]
    public void Location_waits_for_the_whole_head_and_refuses_what_is_not_http()
    {
        Location("GET / HTTP/1.1\r\nHost: lab\r\n").Should().BeNull("the head has not ended yet");
        Location("SSH-2.0-OpenSSH\r\n\r\n").Should().BeNull();
        Location("GET http://elsewhere/ HTTP/1.1\r\n\r\n").Should().BeNull("only a path is redirected");
    }

    [Fact]
    public async Task Plain_http_on_the_https_port_is_redirected_and_https_still_answers()
    {
        var ct = TestContext.Current.CancellationToken;
        using var host = new KoppelFactory(kestrel: true);

        using var tcp = new TcpClient();
        await tcp.ConnectAsync(IPAddress.Loopback, host.HttpPort, ct);
        var stream = tcp.GetStream();
        await stream.WriteAsync(Encoding.ASCII.GetBytes($"GET /devices HTTP/1.1\r\nHost: localhost:{host.HttpPort}\r\n\r\n"), ct);
        using var reader = new StreamReader(stream, Encoding.ASCII);
        var response = await reader.ReadToEndAsync(ct);

        response.Should().StartWith("HTTP/1.1 301 ").And.Contain($"Location: https://localhost:{host.HttpPort}/devices\r\n");

        using var ui = host.CreateAnonymousUiClient();
        (await ui.GetAsync("/api/auth/state", ct)).StatusCode.Should().Be(HttpStatusCode.OK);
    }
}
