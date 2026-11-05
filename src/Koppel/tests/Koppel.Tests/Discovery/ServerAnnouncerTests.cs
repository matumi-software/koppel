using System.Buffers.Binary;
using System.Net;
using System.Net.Sockets;
using Koppel.Discovery;
using Microsoft.Extensions.Logging.Abstractions;
using Skakel.Protocol;

namespace Koppel.Tests.Discovery;

/// <summary>Koppel's own discovery answers, which it sends in Skakel's format while its role is server or both.</summary>
public sealed class ServerAnnouncerTests : IDisposable
{
    private static readonly string Fingerprint = string.Join(':', Enumerable.Repeat("AB", 32));

    private readonly ServerAnnouncer _announcer = new(NullLogger<ServerAnnouncer>.Instance);
    private readonly UdpClient _udp = new(new IPEndPoint(IPAddress.Loopback, 0));
    private readonly IPEndPoint _server;

    public ServerAnnouncerTests()
    {
        _announcer.Announce(new DiscoveryAnswer("bench", 5241, Fingerprint, new SkakelApp("koppel", "1.0.0"), SkakelProtocol.Versions), 0);
        _server = new IPEndPoint(IPAddress.Loopback, _announcer.LocalPort!.Value);
    }

    public void Dispose()
    {
        _udp.Dispose();
        _announcer.Dispose();
    }

    [Fact]
    public async Task A_probe_gets_the_answer()
    {
        var answer = await ProbeAsync(TestContext.Current.CancellationToken);

        answer.Should().NotBeNull();
        SkakelDiscovery.TryParseAnswer(answer!, out var parsed).Should().BeTrue();
        parsed!.Fingerprint.Should().Be(Fingerprint);
    }

    [Fact]
    public async Task A_probe_from_port_0_gets_no_answer_and_the_answers_go_on()
    {
        var ct = TestContext.Current.CancellationToken;
        var probe = SkakelDiscovery.CreateProbe();
        var datagram = new byte[8 + probe.Length];
        BinaryPrimitives.WriteUInt16BigEndian(datagram.AsSpan(0), 0);
        BinaryPrimitives.WriteUInt16BigEndian(datagram.AsSpan(2), (ushort)_server.Port);
        BinaryPrimitives.WriteUInt16BigEndian(datagram.AsSpan(4), (ushort)datagram.Length);
        probe.CopyTo(datagram, 8);
        try
        {
            // A UDP socket cannot send from port 0, so the datagram is built by hand, checksum left out.
            using var raw = new Socket(AddressFamily.InterNetwork, SocketType.Raw, ProtocolType.Udp);
            raw.SendTo(datagram, new IPEndPoint(IPAddress.Loopback, 0));
        }
        catch (SocketException ex)
        {
            Assert.Skip($"Sending a raw UDP datagram is not allowed here: {ex.SocketErrorCode}.");
        }

        await Task.Delay(300, ct);

        (await ProbeAsync(ct)).Should().NotBeNull("one probe that cannot be answered must not stop discovery until a restart");
    }

    [Theory]
    [InlineData("192.168.1.20", 50000, true)]
    [InlineData("127.0.0.1", 50000, true)]
    [InlineData("::ffff:10.0.0.5", 50000, true)]
    [InlineData("fe80::1", 50000, true)]
    [InlineData("192.168.1.20", 0, false)]
    [InlineData("0.0.0.0", 50000, false)]
    [InlineData("255.255.255.255", 50000, false)]
    [InlineData("224.0.0.251", 50000, false)]
    [InlineData("::", 50000, false)]
    [InlineData("ff02::1", 50000, false)]
    public void CanAnswer_takes_only_a_unicast_sender_on_a_real_port(string address, int port, bool answered) =>
        ServerAnnouncer.CanAnswer(new IPEndPoint(IPAddress.Parse(address), port)).Should().Be(answered);

    private async Task<byte[]?> ProbeAsync(CancellationToken ct)
    {
        await _udp.SendAsync(SkakelDiscovery.CreateProbe(), _server, ct);
        using var wait = CancellationTokenSource.CreateLinkedTokenSource(ct);
        wait.CancelAfter(TimeSpan.FromSeconds(1));
        try
        {
            return (await _udp.ReceiveAsync(wait.Token)).Buffer;
        }
        catch (OperationCanceledException) when (!ct.IsCancellationRequested)
        {
            return null;
        }
    }
}
