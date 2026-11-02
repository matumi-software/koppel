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
    public void Keeps_at_most_64_entries_and_still_refreshes_the_ones_it_has()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        for (var i = 0; i < ServerBrowser.MaxEntries + 20; i++)
        {
            browser.Add($"instance-{i}", null, Server($"server-{i:D3}", i, $"10.0.{i / 8}.1"), 0);
        }

        browser.Servers.Should().HaveCount(ServerBrowser.MaxEntries);
        var last = ServerBrowser.MaxEntries + 19;
        browser.Add($"instance-{last}", null, Server("renamed", last, $"10.0.{last / 8}.1"), 0);
        browser.Servers.Should().HaveCount(ServerBrowser.MaxEntries).And.Contain(s => s.Name == "renamed");
    }

    [Fact]
    public void One_address_cannot_fill_the_list_and_only_pushes_out_its_own_entries()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        for (var i = 0; i < 10; i++)
        {
            browser.Add($"real-{i}", null, Server($"real-{i}", i, $"10.0.0.{i + 10}"), 0);
            time.Advance(TimeSpan.FromSeconds(1));
        }

        for (var i = 0; i < 500; i++)
        {
            browser.Add($"squat-{i}", null, Server($"squat-{i}", 1000 + i, "10.0.0.66"), 0);
            time.Advance(TimeSpan.FromSeconds(1));
        }

        var servers = browser.Servers;
        servers.Where(s => s.Address == "10.0.0.66").Should().HaveCount(ServerBrowser.MaxEntriesPerAddress);
        servers.Select(s => s.Name).Should().Contain(Enumerable.Range(0, 10).Select(i => $"real-{i}"), "nothing from another address was evicted");
        servers.Where(s => s.Name.StartsWith("squat-", StringComparison.Ordinal)).Select(s => s.Name)
            .Should().Contain("squat-499", "its newest entries are the ones it keeps");
    }

    [Fact]
    public void A_full_list_makes_room_by_dropping_the_stalest_entry_of_the_address_holding_the_most()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        // 7 addresses with 8 entries each fill 56, and 8 single entries from other addresses fill the rest. The first of those is the oldest of all.
        browser.Add("old-single", null, Server("old-single", 5000, "10.9.9.9"), 0);
        time.Advance(TimeSpan.FromSeconds(1));
        for (var a = 0; a < 7; a++)
        {
            for (var i = 0; i < ServerBrowser.MaxEntriesPerAddress; i++)
            {
                browser.Add($"a{a}-{i}", null, Server($"a{a}-{i}", a * 100 + i, $"10.1.0.{a + 1}"), 0);
                time.Advance(TimeSpan.FromSeconds(1));
            }
        }

        for (var i = 0; i < 7; i++)
        {
            browser.Add($"single-{i}", null, Server($"single-{i}", 6000 + i, $"10.2.0.{i + 1}"), 0);
        }

        browser.Servers.Should().HaveCount(ServerBrowser.MaxEntries);
        browser.Add("newcomer", null, Server("newcomer", 7000, "10.3.0.1"), 0);

        var names = browser.Servers.Select(s => s.Name).ToList();
        names.Should().HaveCount(ServerBrowser.MaxEntries).And.Contain("newcomer").And.Contain("old-single");
        names.Should().NotContain("a0-0", "the stalest entry of a busy address made room");
    }

    [Fact]
    public async Task Publishes_only_when_the_list_changed()
    {
        var time = new ManualTimeProvider();
        var seen = new List<int>();
        using var browser = new ServerBrowser(time);
        browser.Changed += servers =>
        {
            lock (seen)
            {
                seen.Add(servers.Count);
            }

            return Task.CompletedTask;
        };

        browser.Add("a", "eth0", Server("a", 1, "10.0.0.2"), 0);
        await WaitForAsync(() => seen.Count == 1);
        for (var i = 0; i < 20; i++)
        {
            time.Advance(TimeSpan.FromSeconds(10));
            browser.Add("a", "eth0", Server("a", 1, "10.0.0.2"), 0);
        }

        // Anything the 20 refreshes queued would be ahead of this one, in order.
        browser.Add("b", "eth0", Server("b", 2, "10.0.0.3"), 0);
        await WaitForAsync(() => seen.Contains(2));

        lock (seen)
        {
            seen.Should().Equal(1, 2);
        }
    }

    [Fact]
    public void A_server_that_copies_a_name_sits_beside_the_real_one()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        browser.Add("lab-rack-3", "eth0", Server("lab-rack-3", 1, "10.0.0.2"), 0);
        browser.Add("lab-rack-3", "eth0", Server("lab-rack-3", 2, "10.0.0.99"), 0);

        browser.Servers.Should().HaveCount(2, "the same instance name and interface, but another fingerprint")
            .And.OnlyContain(s => s.Name == "lab-rack-3");
        browser.Servers.Select(s => s.Address).Should().BeEquivalentTo("10.0.0.2", "10.0.0.99");
    }

    [Fact]
    public void One_server_seen_on_two_interfaces_is_still_one_entry()
    {
        using var browser = new ServerBrowser(new ManualTimeProvider());
        browser.Add("lab", "eth0", Server("lab", 1, "10.0.0.2"), 0);
        browser.Add("lab", "wlan0", Server("lab", 1, "10.0.0.2"), 1);

        browser.Servers.Should().ContainSingle();
    }

    [Fact]
    public void Drops_entries_nobody_refreshes_and_keeps_the_ones_that_are_refreshed()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        browser.Add("stale", null, Server("stale", 1), 0);
        browser.Add("fresh", null, Server("fresh", 2), 0);

        time.Advance(ServerBrowser.EntryLifetime - TimeSpan.FromMinutes(1));
        browser.Add("fresh", null, Server("fresh", 2), 0);
        time.Advance(TimeSpan.FromMinutes(2));

        browser.Servers.Select(s => s.Name).Should().Equal("fresh");
    }

    [Fact]
    public async Task A_slow_listener_gets_the_latest_list_in_order_and_never_two_at_once()
    {
        var time = new ManualTimeProvider();
        var seen = new List<int>();
        var gate = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var running = 0;
        var overlapped = false;
        using var browser = new ServerBrowser(time);
        browser.Changed += async servers =>
        {
            if (Interlocked.Increment(ref running) > 1)
            {
                overlapped = true;
            }

            if (seen.Count == 0)
            {
                await gate.Task;
            }

            lock (seen)
            {
                seen.Add(servers.Count);
            }

            Interlocked.Decrement(ref running);
        };

        browser.Add("a", null, Server("a", 1), 0);
        await WaitForAsync(() => Volatile.Read(ref running) == 1);
        browser.Add("b", null, Server("b", 2), 0);
        browser.Add("c", null, Server("c", 3), 0);
        browser.Add("d", null, Server("d", 4), 0);
        gate.SetResult();
        await WaitForAsync(() => { lock (seen) { return seen.Contains(4); } });

        lock (seen)
        {
            seen.Should().Equal([1, 4], "the lists in between were dropped for the latest");
        }

        overlapped.Should().BeFalse();
    }

    [Fact]
    public async Task A_role_change_during_a_restart_waits_and_browsing_ends_stopped()
    {
        var time = new ManualTimeProvider();
        var control = new FakeBrowse();
        using var browser = new ServerBrowser(time, null, control);
        browser.Start();
        control.Calls.Clear();
        time.Advance(ServerBrowser.RefreshEvery - TimeSpan.FromSeconds(30));
        var release = new ManualResetEventSlim();
        control.BlockNextStop = release;
        control.InStop = new ManualResetEventSlim();

        // The sweep decides to restart and stops the browse; the role moves to server while it is in there.
        var restart = Task.Run(() => time.Advance(TimeSpan.FromSeconds(30)), TestContext.Current.CancellationToken);
        control.InStop.Wait(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken).Should().BeTrue();
        var stopping = Task.Run(browser.Stop, TestContext.Current.CancellationToken);
        release.Set();
        await Task.WhenAll(restart, stopping).WaitAsync(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken);

        browser.IsBrowsing.Should().BeFalse("the stop came after the restart, not in the middle of it");
        control.Calls.Should().Equal("stop", "start", "stop");
    }

    [Fact]
    public void A_sweep_after_the_role_moved_does_not_start_browsing_again()
    {
        var time = new ManualTimeProvider();
        var control = new FakeBrowse();
        using var browser = new ServerBrowser(time, null, control);
        browser.Start();
        browser.Stop();
        control.Calls.Clear();

        time.Advance(ServerBrowser.RefreshEvery + ServerBrowser.SweepEvery);

        control.Calls.Should().BeEmpty();
        browser.IsBrowsing.Should().BeFalse();
    }

    [Fact]
    public void A_restart_that_throws_anything_does_not_leave_the_timer_thread()
    {
        var time = new ManualTimeProvider();
        var control = new FakeBrowse();
        using var browser = new ServerBrowser(time, null, control);
        browser.Start();
        control.StartThrows = new InvalidOperationException("the browser is not in a state to start");

        var advance = () => time.Advance(ServerBrowser.RefreshEvery + ServerBrowser.SweepEvery);

        advance.Should().NotThrow();
    }

    [Fact]
    public void Browsing_restarts_when_nothing_is_wrong()
    {
        var time = new ManualTimeProvider();
        var control = new FakeBrowse();
        using var browser = new ServerBrowser(time, null, control);
        browser.Start();
        control.Calls.Clear();

        time.Advance(ServerBrowser.RefreshEvery + ServerBrowser.SweepEvery);

        control.Calls.Should().Equal("stop", "start");
        browser.IsBrowsing.Should().BeTrue();
    }

    private static async Task WaitForAsync(Func<bool> condition)
    {
        var deadline = DateTime.UtcNow + TimeSpan.FromSeconds(10);
        while (!condition())
        {
            DateTime.UtcNow.Should().BeBefore(deadline);
            await Task.Delay(10, TestContext.Current.CancellationToken);
        }
    }

    private sealed class FakeBrowse : ServerBrowser.IBrowseControl
    {
        private bool _browsing;

        public List<string> Calls { get; } = [];
        public Exception? StartThrows { get; set; }
        public ManualResetEventSlim? BlockNextStop { get; set; }
        public ManualResetEventSlim? InStop { get; set; }

        public bool IsBrowsing => Volatile.Read(ref _browsing);

        public void StartBrowse()
        {
            if (StartThrows is { } ex)
            {
                StartThrows = null;
                throw ex;
            }

            Calls.Add("start");
            _browsing = true;
        }

        public void StopBrowse()
        {
            if (BlockNextStop is { } gate)
            {
                BlockNextStop = null;
                InStop?.Set();
                gate.Wait(TimeSpan.FromSeconds(10));
            }

            Calls.Add("stop");
            _browsing = false;
        }
    }

    private static DiscoveredServer Server(string name, int n, string address = "10.0.0.2") =>
        new(name, address, 5241, ServerBrowser.NormalizeFingerprint(n.ToString("X", System.Globalization.CultureInfo.InvariantCulture).PadLeft(64, '0'))!, null);

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
