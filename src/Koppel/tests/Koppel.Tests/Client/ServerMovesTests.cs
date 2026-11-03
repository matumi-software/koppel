using System.Net;
using System.Net.Sockets;
using System.Security.Authentication;
using Koppel.Client;
using Koppel.Core.Auth;
using Koppel.Core.Discovery;
using Koppel.Usb;

namespace Koppel.Tests.Client;

/// <summary>Where a client follows its server to, and which failures count as the server being gone from its address.</summary>
public sealed class ServerMovesTests
{
    private const string Current = "3F:9A:1C:2B:7D:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA";
    private const string Next = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";
    private const string Other = "AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA";

    private static readonly ClientConnection Saved = new("https://lab-rack-3:5241", Current, "client-0a1b2c3d4e", "secret");
    private static readonly IPAddress[] LabRack3 = [IPAddress.Parse("10.0.0.5")];

    private static DiscoveredServer Heard(string address, int port, string fingerprint = Current) => new("lab-rack-3", address, port, fingerprint, "0.1.0");

    [Fact]
    public void Pick_prefers_the_saved_host_on_another_port_and_keeps_its_name()
    {
        ServerMoves.Pick(Saved, [Heard("10.0.0.9", 5240), Heard("10.0.0.5", 5240)], LabRack3).Should().Be("https://lab-rack-3:5240");
    }

    [Fact]
    public void Pick_follows_the_server_to_another_address()
    {
        ServerMoves.Pick(Saved, [Heard("10.0.0.9", 5240)], LabRack3).Should().Be("https://10.0.0.9:5240");
    }

    [Fact]
    public void Pick_follows_the_next_certificate_the_server_announced()
    {
        var waiting = Saved with { NextServerFingerprint = Next };

        ServerMoves.Pick(waiting, [Heard("10.0.0.5", 5240, Next)], LabRack3).Should().Be("https://lab-rack-3:5240");
    }

    [Fact]
    public void Pick_ignores_a_server_with_another_fingerprint()
    {
        ServerMoves.Pick(Saved, [Heard("10.0.0.5", 5240, Other), Heard("10.0.0.9", 5240, Other)], LabRack3).Should().BeNull();
    }

    [Fact]
    public void Pick_ignores_the_address_already_saved()
    {
        ServerMoves.Pick(Saved, [Heard("10.0.0.5", 5241)], LabRack3).Should().BeNull("that is where it does not answer");
        ServerMoves.Pick(Saved with { ServerUrl = "https://10.0.0.5:5241" }, [Heard("10.0.0.5", 5241)], []).Should().BeNull();
    }

    [Fact]
    public void Pick_skips_an_address_it_moved_to_a_little_while_ago()
    {
        ServerMoves.Pick(Saved, [Heard("10.0.0.5", 5240), Heard("10.0.0.9", 5240)], LabRack3, url => url == "https://lab-rack-3:5240")
            .Should().Be("https://10.0.0.9:5240");
    }

    [Fact]
    public void IsUnreachable_is_true_when_nothing_answered()
    {
        RemoteFailure.IsUnreachable(new HttpRequestException(HttpRequestError.ConnectionError, "refused", new SocketException((int)SocketError.ConnectionRefused))).Should().BeTrue();
        RemoteFailure.IsUnreachable(new SocketException((int)SocketError.HostUnreachable)).Should().BeTrue();
        RemoteFailure.IsUnreachable(new SocketException((int)SocketError.TimedOut)).Should().BeTrue();
        RemoteFailure.IsUnreachable(new HttpRequestException(HttpRequestError.NameResolutionError, "no such host")).Should().BeTrue();
    }

    [Fact]
    public void IsUnreachable_is_false_when_the_server_answered()
    {
        RemoteFailure.IsUnreachable(new HttpRequestException(HttpRequestError.SecureConnectionError, "tls", new AuthenticationException())).Should().BeFalse("a certificate mismatch is not a move");
        RemoteFailure.IsUnreachable(new TokenRequestException("invalid_client", null)).Should().BeFalse("refused credentials are not a move");
        RemoteFailure.IsUnreachable(new InvalidOperationException("The server closed the link.")).Should().BeFalse();
    }
}
