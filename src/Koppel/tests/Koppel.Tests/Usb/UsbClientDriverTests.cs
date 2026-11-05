using System.Globalization;
using System.Net;
using System.Net.Http.Json;
using System.Net.Sockets;
using System.Text.Json;
using Koppel.Core.Activity;
using Koppel.Core.Usb;
using Koppel.Usb;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;
using Skakel.Server;

namespace Koppel.Tests.Usb;

/// <summary>
/// The client on Windows, where a driver on the machine makes its own connection to Koppel's gateway:
/// what the gateway serves, what happens when the user changes their mind while the driver is working,
/// and how the link keeps itself alive across the server's token expiry. A fake driver plays usbip-win2.
/// </summary>
public sealed class UsbClientDriverTests : IDisposable
{
    private readonly FakeKernel _serverKernel = new();
    private readonly FakeUsbIpDriver _driver = new();
    private readonly int _gatewayPort = KoppelFactory.FreePort();
    private readonly KoppelFactory _server;
    private KoppelFactory? _client;

    public UsbClientDriverTests()
    {
        _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_serverKernel), shareDongle: true);
    }

    public void Dispose()
    {
        _client?.Dispose();
        _server.Dispose();
        _driver.Dispose();
    }

    private KoppelFactory NewClient(Action<IServiceCollection>? services = null, int? maxSessions = null)
    {
        var configuration = new Dictionary<string, string> { ["Gateway:Port"] = _gatewayPort.ToString(CultureInfo.InvariantCulture) };
        if (maxSessions is { } max)
        {
            configuration["Gateway:MaxSessions"] = max.ToString(CultureInfo.InvariantCulture);
        }

        return _client = new KoppelFactory(
            kestrel: true,
            services: s =>
            {
                s.AddSingleton<IUsbIpDriver>(_driver);
                services?.Invoke(s);
            },
            configuration: configuration);
    }

    [Fact]
    public async Task Detaching_while_the_driver_is_still_attaching_releases_what_it_makes()
    {
        var ct = TestContext.Current.CancellationToken;
        var client = NewClient();
        _driver.AttachGate = new TaskCompletionSource();
        await using var hub = await LinkAsync(client, ct);
        using var ui = client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        await _driver.Connection.Task.WaitAsync(TimeSpan.FromSeconds(10), ct);

        // usbip.exe has not returned yet, so Koppel does not know the driver's port.
        (await ui.PostAsync("/api/client/devices/1-1.4/detach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        _driver.Detached.Should().BeEmpty();
        _driver.AttachGate.SetResult();

        await EventuallyAsync(() => _driver.Detached.Count == 1, ct);
        _driver.Detached.Should().Equal("1-1.4@1");
        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/client/devices", ct);
        devices.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean().Should().BeFalse();
        File.ReadAllText(client.ConfigFile("attached.json")).Should().NotContain("1-1.4");

        // Nothing is left for a stop to release, so the driver hears of this device once.
        await client.Services.GetRequiredService<UsbClient>().StopAsync(ct);
        _driver.Detached.Should().Equal("1-1.4@1");
    }

    [Fact]
    public async Task An_import_nobody_asked_the_driver_for_is_refused()
    {
        var ct = TestContext.Current.CancellationToken;
        var client = NewClient();
        await using var hub = await LinkAsync(client, ct);
        var usb = client.Services.GetRequiredService<UsbClient>();
        await EventuallyAsync(() => usb.GatewayPort is not null, ct);

        // Another user's "usbip attach", or the driver trying again by itself: the kernel connects, Koppel did not ask.
        using var visitor = new TcpClient();
        await visitor.ConnectAsync(IPAddress.Loopback, _gatewayPort, ct);
        var stream = visitor.GetStream();
        await stream.WriteAsync(UsbIpProtocol.ImportRequest("1-1.4"), ct);
        var reply = await UsbIpProtocol.ReadHeaderAsync(stream, ct);
        reply.Status.Should().NotBe(UsbIpProtocol.StatusOk);

        using var list = new TcpClient();
        await list.ConnectAsync(IPAddress.Loopback, _gatewayPort, ct);
        await list.GetStream().WriteAsync(UsbIpProtocol.DevListRequest(), ct);
        (await UsbIpProtocol.ReadHeaderAsync(list.GetStream(), ct)).Status.Should().NotBe(UsbIpProtocol.StatusOk);

        using var ui = client.CreateUiClient();
        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/client/devices", ct);
        devices.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean().Should().BeFalse();
        _driver.Attaches.Should().Be(0);
    }

    [Fact]
    public async Task A_remembered_device_is_attached_again_after_the_link_drops_because_koppel_asks_for_it()
    {
        var ct = TestContext.Current.CancellationToken;
        var client = NewClient();
        await using var hub = await LinkAsync(client, ct);
        using var ui = client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        await EventuallyAsync(() => _driver.Attaches == 1 && AttachedHere(client), ct);

        // The connection goes away; the driver would try again by itself and is refused, Koppel asks anew.
        _driver.DropConnection();

        await EventuallyAsync(() => _driver.Attaches == 2 && AttachedHere(client), ct);
    }

    [Fact]
    public async Task A_role_change_returns_at_once_while_the_driver_is_still_detaching()
    {
        var ct = TestContext.Current.CancellationToken;
        var client = NewClient();
        await using var hub = await LinkAsync(client, ct);
        using var ui = client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        await EventuallyAsync(() => AttachedHere(client), ct);
        _driver.DetachGate = new TaskCompletionSource();

        // usbip.exe may take 30 seconds; the settings update, and the confirmation the UI waits for, must not.
        var change = ui.PatchAsJsonAsync("/api/settings", new { role = "server" }, ct);
        (await change.WaitAsync(TimeSpan.FromSeconds(10), ct)).EnsureSuccessStatusCode();
        await EventuallyAsync(() => _driver.Detached.Count == 1, ct);

        _driver.DetachGate.SetResult();
        await EventuallyAsync(() => !AttachedHere(client), ct);
    }

    [Fact]
    public async Task The_gateway_serves_at_most_the_configured_number_of_sessions()
    {
        var ct = TestContext.Current.CancellationToken;
        var client = NewClient(maxSessions: 2);
        await using var hub = await LinkAsync(client, ct);
        var usb = client.Services.GetRequiredService<UsbClient>();
        await EventuallyAsync(() => usb.GatewayPort is not null, ct);

        // Two connections that say nothing hold both places.
        using var first = new TcpClient();
        using var second = new TcpClient();
        await first.ConnectAsync(IPAddress.Loopback, _gatewayPort, ct);
        await second.ConnectAsync(IPAddress.Loopback, _gatewayPort, ct);
        await EventuallyAsync(async () =>
        {
            using var third = new TcpClient();
            await third.ConnectAsync(IPAddress.Loopback, _gatewayPort, ct);
            return await ReadsToEndAsync(third.GetStream(), ct);
        }, ct);

        first.Connected.Should().BeTrue();
    }

    [Fact]
    public async Task The_link_is_made_again_before_its_token_ends_without_a_failure_or_a_connecting_flash()
    {
        var ct = TestContext.Current.CancellationToken;
        var clock = new ManualTimeProvider();
        var client = NewClient(s => s.AddSingleton<TimeProvider>(clock));
        await using var hub = await client.ConnectUiHubAsync(ct);
        var links = new List<string>();
        hub.On<JsonElement>("ClientDevicesChanged", d => { lock (links) { links.Add(d.GetProperty("link").GetString()!); } });
        var linked = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        await Link.ConnectAsync(_server, client, ct);
        await Link.WithLastAsync(linked, hub, ct);
        // The server notes when a client last took a token; the renewal takes a new one.
        var clients = _server.Services.GetRequiredService<SkakelClients>();
        var first = clients.All.Should().ContainSingle().Subject.LastSeen;
        lock (links)
        {
            links.Clear();
        }

        // The server's token lasts 30 minutes; the client renews half a minute ahead.
        clock.Advance(TimeSpan.FromMinutes(29) + TimeSpan.FromSeconds(45));

        await EventuallyAsync(() => clients.All is [{ LastSeen: var seen }] && seen > first, ct);
        await EventuallyAsync(() => client.Services.GetRequiredService<UsbClient>().Current.Link == "connected", ct);
        lock (links)
        {
            links.Should().NotContain(l => l != "connected");
        }

        client.Services.GetRequiredService<IActivityReader>().Recent(50)
            .Should().NotContain(e => e.Reason == ActivityReason.LinkDown);
    }

    private static bool AttachedHere(KoppelFactory client) =>
        client.Services.GetRequiredService<UsbClient>().Current.Devices is [{ AttachedHere: true }];

    private static async Task<bool> ReadsToEndAsync(NetworkStream stream, CancellationToken ct)
    {
        var buffer = new byte[1];
        try
        {
            return await stream.ReadAsync(buffer, ct).AsTask().WaitAsync(TimeSpan.FromSeconds(2), ct) == 0;
        }
        catch (TimeoutException)
        {
            return false;
        }
        catch (IOException)
        {
            return true;
        }
    }

    private async Task<HubConnection> LinkAsync(KoppelFactory client, CancellationToken ct)
    {
        var hub = await client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        await Link.ConnectAsync(_server, client, ct);
        await Link.WithLastAsync(linked, hub, ct);
        return hub;
    }

    private static Task EventuallyAsync(Func<bool> condition, CancellationToken ct) => EventuallyAsync(() => Task.FromResult(condition()), ct);

    private static async Task EventuallyAsync(Func<Task<bool>> condition, CancellationToken ct)
    {
        var until = DateTime.UtcNow.AddSeconds(30);
        while (!await condition())
        {
            if (DateTime.UtcNow > until)
            {
                throw new TimeoutException("The condition did not hold in 30 seconds.");
            }

            await Task.Delay(100, ct);
        }
    }
}
