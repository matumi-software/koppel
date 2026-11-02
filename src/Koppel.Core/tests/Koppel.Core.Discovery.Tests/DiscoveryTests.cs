using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Core.Discovery.Tests;

public sealed class DiscoveryTests
{
    private const string Fingerprint = "EF:8E:16:AF:C3:88:BF:8C:94:CB:98:9A:3F:6A:93:F9:3D:0B:30:60:33:87:79:CF:C0:A4:E6:A4:30:89:14:29";

    [Fact]
    public void Reads_an_answer_and_takes_the_address_it_came_from()
    {
        var answer = KoppelService.CreateReply(new ServerAnnouncement("t520-client", 5241, Fingerprint, "0.1.0"));

        var server = ServerBrowser.Read(answer, IPAddress.Parse("192.168.31.122"));

        server.Should().Be(new DiscoveredServer("t520-client", "192.168.31.122", 5241, Fingerprint, "0.1.0"));
        server!.Url.Should().Be("https://192.168.31.122:5241");
        answer.Length.Should().BeLessThan(KoppelService.ProbeSize, "an answer never outgrows the probe it answers");
    }

    [Fact]
    public void Ignores_answers_without_a_fingerprint_or_from_another_protocol_version()
    {
        var from = IPAddress.Parse("10.0.0.2");
        ServerBrowser.Read(Answer("""{"name":"x","port":5241}"""), from).Should().BeNull();
        ServerBrowser.Read(Encoding.UTF8.GetBytes($"KOPPEL-SERVER/2\n{{\"port\":5241,\"fingerprint\":\"{Fingerprint}\"}}"), from).Should().BeNull();
        ServerBrowser.Read(Answer("not json"), from).Should().BeNull();
        ServerBrowser.Read(Answer($$"""{"port":70000,"fingerprint":"{{Fingerprint}}"}"""), from).Should().BeNull();
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
        ServerBrowser.Read(Answer(fingerprint: fingerprint), IPAddress.Parse("10.0.0.2")).Should().BeNull();
    }

    [Fact]
    public void Writes_fingerprints_the_way_the_servers_clients_page_does()
    {
        var plain = Fingerprint.Replace(":", "", StringComparison.Ordinal).ToLowerInvariant();

        ServerBrowser.Read(Answer(fingerprint: plain), IPAddress.Parse("10.0.0.2"))!.Fingerprint.Should().Be(Fingerprint);
    }

    [Fact]
    public void Bounds_the_name_and_version_and_drops_control_characters()
    {
        var server = ServerBrowser.Read(Answer(name: new string('n', 200), version: new string('9', 200)), IPAddress.Parse("10.0.0.2"))!;

        server.Name.Should().HaveLength(ServerBrowser.MaxNameLength);
        server.Version.Should().HaveLength(ServerBrowser.MaxVersionLength);
        ServerBrowser.Read(Answer(name: "a\u0007b\nc"), IPAddress.Parse("10.0.0.2"))!.Name.Should().Be("abc");
    }

    [Fact]
    public async Task A_server_answers_a_full_size_probe_but_not_a_short_one()
    {
        var ct = TestContext.Current.CancellationToken;
        var port = FreeUdpPort();
        using var announcer = new ServerAnnouncer(NullLogger<ServerAnnouncer>.Instance);
        announcer.Announce(new ServerAnnouncement("koppel-test", port, Fingerprint, "0.1.0"));
        using var client = new UdpClient(new IPEndPoint(IPAddress.Loopback, 0));
        var server = new IPEndPoint(IPAddress.Loopback, port);

        // Short probes would let anyone with a spoofed address make a server send them more than they sent.
        await client.SendAsync(KoppelService.CreateProbe().AsMemory(0, 64), server, ct);
        var answer = client.ReceiveAsync(ct).AsTask();
        (await Task.WhenAny(answer, Task.Delay(500, ct))).Should().NotBeSameAs(answer);

        await client.SendAsync(KoppelService.CreateProbe(), server, ct);
        var received = await answer.WaitAsync(TimeSpan.FromSeconds(5), ct);
        ServerBrowser.Read(received.Buffer, received.RemoteEndPoint.Address)!.Fingerprint.Should().Be(Fingerprint);
    }

    [Fact]
    public void Keeps_at_most_64_entries_and_still_refreshes_the_ones_it_has()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        for (var i = 0; i < ServerBrowser.MaxEntries + 20; i++)
        {
            browser.Add(Server($"server-{i:D3}", i, $"10.0.{i / 8}.1"), 0);
        }

        browser.Servers.Should().HaveCount(ServerBrowser.MaxEntries);
        var last = ServerBrowser.MaxEntries + 19;
        browser.Add(Server("renamed", last, $"10.0.{last / 8}.1"), 0);
        browser.Servers.Should().HaveCount(ServerBrowser.MaxEntries).And.Contain(s => s.Name == "renamed");
    }

    [Fact]
    public void One_address_cannot_fill_the_list_and_only_pushes_out_its_own_entries()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        for (var i = 0; i < 10; i++)
        {
            browser.Add(Server($"real-{i}", i, $"10.0.0.{i + 10}"), 0);
            time.Advance(TimeSpan.FromSeconds(1));
        }

        for (var i = 0; i < 500; i++)
        {
            browser.Add(Server($"squat-{i}", 1000 + i, "10.0.0.66"), 0);
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
        browser.Add(Server("old-single", 5000, "10.9.9.9"), 0);
        time.Advance(TimeSpan.FromSeconds(1));
        for (var a = 0; a < 7; a++)
        {
            for (var i = 0; i < ServerBrowser.MaxEntriesPerAddress; i++)
            {
                browser.Add(Server($"a{a}-{i}", a * 100 + i, $"10.1.0.{a + 1}"), 0);
                time.Advance(TimeSpan.FromSeconds(1));
            }
        }

        for (var i = 0; i < 7; i++)
        {
            browser.Add(Server($"single-{i}", 6000 + i, $"10.2.0.{i + 1}"), 0);
        }

        browser.Servers.Should().HaveCount(ServerBrowser.MaxEntries);
        browser.Add(Server("newcomer", 7000, "10.3.0.1"), 0);

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

        browser.Add(Server("a", 1, "10.0.0.2"), 0);
        await WaitForAsync(() => seen.Count == 1);
        for (var i = 0; i < 20; i++)
        {
            time.Advance(TimeSpan.FromSeconds(10));
            browser.Add(Server("a", 1, "10.0.0.2"), 0);
        }

        // Anything the 20 refreshes queued would be ahead of this one, in order.
        browser.Add(Server("b", 2, "10.0.0.3"), 0);
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
        browser.Add(Server("lab-rack-3", 1, "10.0.0.2"), 0);
        browser.Add(Server("lab-rack-3", 2, "10.0.0.99"), 0);

        browser.Servers.Should().HaveCount(2, "the same name, but another fingerprint")
            .And.OnlyContain(s => s.Name == "lab-rack-3");
        browser.Servers.Select(s => s.Address).Should().BeEquivalentTo("10.0.0.2", "10.0.0.99");
    }

    [Fact]
    public void One_server_answering_twice_is_still_one_entry()
    {
        using var browser = new ServerBrowser(new ManualTimeProvider());
        browser.Add(Server("lab", 1, "10.0.0.2"), 0);
        browser.Add(Server("lab", 1, "10.0.0.2"), 1);

        browser.Servers.Should().ContainSingle();
    }

    [Fact]
    public void Drops_entries_nobody_refreshes_and_keeps_the_ones_that_are_refreshed()
    {
        var time = new ManualTimeProvider();
        using var browser = new ServerBrowser(time);
        browser.Add(Server("stale", 1), 0);
        browser.Add(Server("fresh", 2), 0);

        time.Advance(browser.EntryLifetime - TimeSpan.FromSeconds(5));
        browser.Add(Server("fresh", 2), 0);
        time.Advance(TimeSpan.FromSeconds(20));

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

        browser.Add(Server("a", 1), 0);
        await WaitForAsync(() => Volatile.Read(ref running) == 1);
        browser.Add(Server("b", 2), 0);
        browser.Add(Server("c", 3), 0);
        browser.Add(Server("d", 4), 0);
        gate.SetResult();
        await WaitForAsync(() => { lock (seen) { return seen.Contains(4); } });

        lock (seen)
        {
            seen.Should().Equal([1, 4], "the lists in between were dropped for the latest");
        }

        overlapped.Should().BeFalse();
    }

    [Fact]
    public async Task A_role_change_during_a_probe_waits_and_browsing_ends_stopped()
    {
        var time = new ManualTimeProvider();
        var control = new FakeBrowse();
        using var browser = new ServerBrowser(time, null, control);
        browser.Start();
        control.Calls.Clear();
        var release = new ManualResetEventSlim();
        control.BlockNextProbe = release;
        control.InProbe = new ManualResetEventSlim();

        // The tick is in the middle of a probe; the role moves to server while it is in there.
        var tick = Task.Run(() => time.Advance(ServerBrowser.ProbeEvery), TestContext.Current.CancellationToken);
        control.InProbe.Wait(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken).Should().BeTrue();
        var stopping = Task.Run(browser.Stop, TestContext.Current.CancellationToken);
        release.Set();
        await Task.WhenAll(tick, stopping).WaitAsync(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken);

        browser.IsBrowsing.Should().BeFalse("the stop came after the probe, not in the middle of it");
        control.Calls.Should().Equal("probe", "stop");
    }

    [Fact]
    public void A_tick_after_the_role_moved_does_not_probe_or_start_browsing_again()
    {
        var time = new ManualTimeProvider();
        var control = new FakeBrowse();
        using var browser = new ServerBrowser(time, null, control);
        browser.Start();
        browser.Stop();
        control.Calls.Clear();

        time.Advance(ServerBrowser.ProbeEvery * 3);

        control.Calls.Should().BeEmpty();
        browser.IsBrowsing.Should().BeFalse();
    }

    [Fact]
    public void A_probe_that_throws_anything_does_not_leave_the_timer_thread()
    {
        var time = new ManualTimeProvider();
        var control = new FakeBrowse();
        using var browser = new ServerBrowser(time, null, control);
        browser.Start();
        control.ProbeThrows = new InvalidOperationException("the network is not in a state to send");

        var advance = () => time.Advance(ServerBrowser.ProbeEvery);

        advance.Should().NotThrow();
    }

    [Fact]
    public void A_probe_goes_out_at_the_start_and_on_every_tick_while_browsing()
    {
        var time = new ManualTimeProvider();
        var control = new FakeBrowse();
        using var browser = new ServerBrowser(time, null, control);

        browser.Start();
        time.Advance(ServerBrowser.ProbeEvery * 2);

        control.Calls.Should().Equal("start", "probe", "probe", "probe");
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
        public Exception? ProbeThrows { get; set; }
        public ManualResetEventSlim? BlockNextProbe { get; set; }
        public ManualResetEventSlim? InProbe { get; set; }

        public bool IsBrowsing => Volatile.Read(ref _browsing);

        public void StartBrowse(Action<DiscoveredServer, int> found)
        {
            Calls.Add("start");
            _browsing = true;
        }

        public void StopBrowse()
        {
            Calls.Add("stop");
            _browsing = false;
        }

        public void Probe()
        {
            if (ProbeThrows is { } ex)
            {
                ProbeThrows = null;
                throw ex;
            }

            if (BlockNextProbe is { } gate)
            {
                BlockNextProbe = null;
                InProbe?.Set();
                gate.Wait(TimeSpan.FromSeconds(10));
            }

            Calls.Add("probe");
        }
    }

    private static DiscoveredServer Server(string name, int n, string address = "10.0.0.2") =>
        new(name, address, 5241, ServerBrowser.NormalizeFingerprint(n.ToString("X", System.Globalization.CultureInfo.InvariantCulture).PadLeft(64, '0'))!, null);

    [Fact]
    public void Writes_IPv6_addresses_in_brackets()
    {
        new DiscoveredServer("x", "2001:db8::5", 5241, Fingerprint, null).Url.Should().Be("https://[2001:db8::5]:5241");
    }

    /// <summary>A real round trip over loopback: probe, answer, and the entry going once the server stops answering.</summary>
    [Fact]
    public async Task A_browser_finds_a_server_and_drops_it_when_it_stops_answering()
    {
        var ct = TestContext.Current.CancellationToken;
        var port = FreeUdpPort();
        using var browser = new ServerBrowser(null, null, ServerBrowser.Udp(port, [new IPEndPoint(IPAddress.Loopback, port)]), TimeSpan.FromMilliseconds(200));
        var found = new TaskCompletionSource<DiscoveredServer>(TaskCreationOptions.RunContinuationsAsynchronously);
        var gone = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        browser.Changed += servers =>
        {
            if (servers.FirstOrDefault(s => s.Name == "koppel-test") is { } server)
            {
                found.TrySetResult(server);
            }
            else if (found.Task.IsCompleted)
            {
                gone.TrySetResult();
            }

            return Task.CompletedTask;
        };

        using var announcer = new ServerAnnouncer(NullLogger<ServerAnnouncer>.Instance);
        announcer.Announce(new ServerAnnouncement("koppel-test", port, Fingerprint, "0.1.0"));
        browser.Start();

        var server = await found.Task.WaitAsync(TimeSpan.FromSeconds(10), ct);
        server.Address.Should().Be("127.0.0.1");
        server.HttpsPort.Should().Be(port);
        server.Fingerprint.Should().Be(Fingerprint);

        announcer.Stop();
        await gone.Task.WaitAsync(TimeSpan.FromSeconds(10), ct);
    }

    private static byte[] Answer(string json) => Encoding.UTF8.GetBytes("KOPPEL-SERVER/1\n" + json);

    private static byte[] Answer(string? name = "x", string fingerprint = Fingerprint, string? version = null) =>
        Answer(JsonSerializer.Serialize(new { name, port = 5241, fingerprint, version }));

    private static int FreeUdpPort()
    {
        using var socket = new Socket(AddressFamily.InterNetwork, SocketType.Dgram, ProtocolType.Udp);
        socket.Bind(new IPEndPoint(IPAddress.Any, 0));
        return ((IPEndPoint)socket.LocalEndPoint!).Port;
    }
}
