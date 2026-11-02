using System.Net;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Core.Discovery.Tests;

public sealed class DiscoveryTests
{
    private const string Fingerprint = "EF:8E:16:AF:C3:88:BF:8C:94:CB:98:9A:3F:6A:93:F9:3D:0B:30:60:33:87:79:CF:C0:A4:E6:A4:30:89:14:29";

    [Fact]
    public void Reads_a_Koppel_announcement()
    {
        var server = ServerBrowser.Read(
            ["v=1", "name=t520-client", $"fp={Fingerprint}", "version=0.1.0"],
            [IPAddress.Parse("172.17.0.1"), IPAddress.Parse("fe80::1"), IPAddress.Parse("192.168.31.122")],
            5241);

        server.Should().NotBeNull();
        server!.Name.Should().Be("t520-client");
        server.Fingerprint.Should().Be(Fingerprint);
        server.Version.Should().Be("0.1.0");
        server.Address.Should().Be("172.17.0.1", "with no interface to compare against, the first IPv4 address wins");
        server.Url.Should().Be("https://172.17.0.1:5241");
    }

    [Fact]
    public void Ignores_announcements_without_a_fingerprint_or_from_another_protocol_version()
    {
        ServerBrowser.Read(["v=1", "name=x"], [IPAddress.Loopback, IPAddress.Parse("10.0.0.2")], 5241).Should().BeNull();
        ServerBrowser.Read(["v=2", $"fp={Fingerprint}"], [IPAddress.Parse("10.0.0.2")], 5241).Should().BeNull();
    }

    [Fact]
    public void Writes_IPv6_addresses_in_brackets()
    {
        new DiscoveredServer("x", "2001:db8::5", 5241, Fingerprint, null).Url.Should().Be("https://[2001:db8::5]:5241");
    }

    /// <summary>A real round trip over this machine's network interfaces, both libraries talking mDNS.</summary>
    [Fact]
    public async Task A_browser_finds_an_announced_server_and_sees_it_leave()
    {
        var ct = TestContext.Current.CancellationToken;
        var name = $"koppel-test-{Guid.NewGuid():N}"[..24];
        using var browser = new ServerBrowser();
        var found = new TaskCompletionSource<DiscoveredServer>(TaskCreationOptions.RunContinuationsAsynchronously);
        var gone = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        browser.Changed += servers =>
        {
            if (servers.FirstOrDefault(s => s.Name == name) is { } server)
            {
                found.TrySetResult(server);
            }
            else if (found.Task.IsCompleted)
            {
                gone.TrySetResult();
            }

            return Task.CompletedTask;
        };
        browser.Start();

        using var announcer = new ServerAnnouncer(NullLogger<ServerAnnouncer>.Instance);
        announcer.Announce(new ServerAnnouncement(name, 5999, Fingerprint, "0.1.0"));

        var server = await found.Task.WaitAsync(TimeSpan.FromSeconds(15), ct);
        server.HttpsPort.Should().Be(5999);
        server.Fingerprint.Should().Be(Fingerprint);

        announcer.Stop();
        await gone.Task.WaitAsync(TimeSpan.FromSeconds(15), ct);
    }
}
