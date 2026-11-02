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

    [Theory]
    [InlineData("")]
    [InlineData("EF:8E:16")]
    [InlineData("EF:8E:16:AF:C3:88:BF:8C:94:CB:98:9A:3F:6A:93:F9:3D:0B:30:60:33:87:79:CF:C0:A4:E6:A4:30:89:14")]
    [InlineData("EF:8E:16:AF:C3:88:BF:8C:94:CB:98:9A:3F:6A:93:F9:3D:0B:30:60:33:87:79:CF:C0:A4:E6:A4:30:89:14:29:00")]
    [InlineData("ZZ:8E:16:AF:C3:88:BF:8C:94:CB:98:9A:3F:6A:93:F9:3D:0B:30:60:33:87:79:CF:C0:A4:E6:A4:30:89:14:29")]
    [InlineData("<script>")]
    public void Rejects_fingerprints_that_are_not_exactly_64_hex_digits(string fingerprint)
    {
        ServerBrowser.Read(["v=1", $"fp={fingerprint}"], [IPAddress.Parse("10.0.0.2")], 5241).Should().BeNull();
    }

    [Fact]
    public void Writes_fingerprints_the_way_the_servers_clients_page_does()
    {
        var plain = Fingerprint.Replace(":", "", StringComparison.Ordinal).ToLowerInvariant();

        ServerBrowser.Read(["v=1", $"fp={plain}"], [IPAddress.Parse("10.0.0.2")], 5241)!.Fingerprint.Should().Be(Fingerprint);
    }

    [Fact]
    public void Bounds_the_name_and_version_and_drops_control_characters()
    {
        var server = ServerBrowser.Read(["v=1", $"fp={Fingerprint}", $"name={new string('n', 200)}", $"version={new string('9', 200)}"], [IPAddress.Parse("10.0.0.2")], 5241)!;

        server.Name.Should().HaveLength(ServerBrowser.MaxNameLength);
        server.Version.Should().HaveLength(ServerBrowser.MaxVersionLength);
        ServerBrowser.Read(["v=1", $"fp={Fingerprint}", "name=a\u0007b\nc"], [IPAddress.Parse("10.0.0.2")], 5241)!.Name.Should().Be("abc");
    }

    [Fact]
    public void Keeps_at_most_64_entries_but_still_refreshes_the_ones_it_has()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        for (var i = 0; i < ServerBrowser.MaxEntries + 20; i++)
        {
            browser.Add($"instance-{i}", Server($"server-{i:D3}", i), 0);
        }

        browser.Servers.Should().HaveCount(ServerBrowser.MaxEntries);
        browser.Add("instance-0", Server("renamed", 0), 0);
        browser.Servers.Should().HaveCount(ServerBrowser.MaxEntries).And.Contain(s => s.Name == "renamed");
    }

    [Fact]
    public void Drops_entries_nobody_refreshes_and_keeps_the_ones_that_are_refreshed()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        browser.Add("stale", Server("stale", 1), 0);
        browser.Add("fresh", Server("fresh", 2), 0);

        time.Advance(ServerBrowser.EntryLifetime - TimeSpan.FromMinutes(1));
        browser.Add("fresh", Server("fresh", 2), 0);
        time.Advance(TimeSpan.FromMinutes(2));

        browser.Servers.Select(s => s.Name).Should().Equal("fresh");
    }

    [Fact]
    public async Task Raises_changes_one_at_a_time_in_the_order_they_happened()
    {
        var time = new ManualTimeProvider();
        var seen = new List<int>();
        var gate = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var all = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        using var browser = new ServerBrowser(time);
        browser.Changed += async servers =>
        {
            if (seen.Count == 0)
            {
                await gate.Task;
            }

            seen.Add(servers.Count);
            if (seen.Count == 3)
            {
                all.SetResult();
            }
        };

        browser.Add("a", Server("a", 1), 0);
        browser.Add("b", Server("b", 2), 0);
        browser.Add("c", Server("c", 3), 0);
        gate.SetResult();
        await all.Task.WaitAsync(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken);

        seen.Should().Equal(1, 2, 3);
    }

    private static DiscoveredServer Server(string name, int n) =>
        new(name, "10.0.0.2", 5241, ServerBrowser.NormalizeFingerprint(n.ToString("X", System.Globalization.CultureInfo.InvariantCulture).PadLeft(64, '0'))!, null);

    [Fact]
    public void Writes_IPv6_addresses_in_brackets()
    {
        new DiscoveredServer("x", "2001:db8::5", 5241, Fingerprint, null).Url.Should().Be("https://[2001:db8::5]:5241");
    }

    /// <summary>
    /// A real round trip over this machine's network interfaces, both libraries talking mDNS. It needs
    /// multicast, so it is tagged Integration: <c>dotnet test --filter "Category!=Integration"</c> skips it.
    /// </summary>
    [Fact]
    [Trait("Category", "Integration")]
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
