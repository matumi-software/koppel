using System.Globalization;
using System.Net;
using System.Net.Http.Json;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Authentication;
using System.Text.Json;
using Koppel.Core.Usb;
using Koppel.Usb;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// What the client does when the server, the link or its own settings misbehave: faults the user can
/// read, a deadline, no restart for an unchanged setting, detaching on a change, and a gateway that
/// cannot bind. Same two real hosts and fake kernels as the end-to-end tests.
/// </summary>
public sealed class UsbClientFailureTests : IDisposable
{
    private readonly FakeKernel _serverKernel = new();
    private readonly FakeKernel _clientKernel = new();
    private readonly KoppelFactory _server;
    private readonly KoppelFactory _client;

    public UsbClientFailureTests()
    {
        _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_serverKernel), shareDongle: true);
        _client = new KoppelFactory(
            kestrel: true,
            services: s => s.AddSingleton<IUsbAttacher>(_clientKernel),
            configuration: new Dictionary<string, string>
            {
                ["Gateway:Port"] = KoppelFactory.FreePort().ToString(CultureInfo.InvariantCulture),
                ["Usb:AttachTimeoutSeconds"] = "3",
            },
            role: "client");
    }

    public void Dispose()
    {
        _client.Dispose();
        _server.Dispose();
    }

    [Fact]
    public async Task Attach_to_a_server_that_went_away_records_a_fault_and_pushes_it()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        var faulted = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("devices")[0].GetProperty("fault").ValueKind == JsonValueKind.String);

        // The token endpoint is gone, as when the server is switched off.
        _server.Dispose();
        using var ui = _client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        var pushed = await Link.WithLastAsync(faulted, hub, ct);
        pushed.GetProperty("devices")[0].GetProperty("fault").GetString().Should().MatchRegex("Could not reach the server|did not answer within");
        pushed.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean().Should().BeFalse();
    }

    [Fact]
    public async Task Attach_to_a_server_that_stalls_gives_up_at_the_deadline()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        using var gate = new ManualResetEventSlim(false);
        _serverKernel.ResetGate = gate;
        var faulted = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("devices")[0].GetProperty("fault").ValueKind == JsonValueKind.String);
        try
        {
            using var ui = _client.CreateUiClient();
            (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

            // The server authenticates the client and then says nothing.
            var pushed = await Link.WithLastAsync(faulted, hub, ct);
            pushed.GetProperty("devices")[0].GetProperty("fault").GetString().Should().Contain("did not answer within 3 seconds");
        }
        finally
        {
            gate.Set();
        }
    }

    [Fact]
    public async Task Attaches_and_the_link_share_one_token_client_until_the_connection_changes()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        var usb = _client.Services.GetRequiredService<UsbClient>();
        var before = usb.ServerLinkFor();

        await AttachAsync(hub, ct);
        usb.ServerLinkFor().Should().BeSameAs(before);
        usb.ServerLinkFor().Tokens.Should().BeSameAs(before.Tokens);

        // New credentials are a new connection, so they get their own token client.
        await Link.ConnectAsync(_server, _client, ct);
        usb.ServerLinkFor().Should().NotBeSameAs(before);
    }

    [Fact]
    public async Task Changing_the_connection_detaches_what_was_attached()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        await AttachAsync(hub, ct);
        _clientKernel.DetachedPorts.Should().BeEmpty();

        await Link.ConnectAsync(_server, _client, ct);

        await EventuallyAsync(() => !_clientKernel.DetachedPorts.IsEmpty, ct);
        _clientKernel.DetachedPorts.Should().Contain(8);
    }

    [Fact]
    public async Task A_setting_that_did_not_change_the_link_leaves_it_and_its_devices_alone()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        await AttachAsync(hub, ct);
        var usb = _client.Services.GetRequiredService<UsbClient>();
        var applied = usb.ChangesApplied;

        using var ui = _client.CreateUiClient();
        (await ui.PatchAsJsonAsync("/api/settings", new { shareOnStartup = false }, ct)).EnsureSuccessStatusCode();

        // The update returns after every handler has run, so an applied change would be counted by now.
        usb.ChangesApplied.Should().Be(applied, "the link keeps running when neither the role nor the connection changed");
        _clientKernel.DetachedPorts.Should().BeEmpty();
        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/client/devices", ct);
        devices.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean().Should().BeTrue();
    }

    [Fact]
    public async Task Detach_and_attach_again_leaves_the_new_attachment_in_place()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        await AttachAsync(hub, ct);
        using var ui = _client.CreateUiClient();

        // The old attach's relay ends while the next one is being set up. It may only clean up its own
        // entry, or it would remove the new attachment and detach the new port (the fake kernel hands out
        // port 8 every time).
        const int cycles = 3;
        for (var cycle = 1; cycle <= cycles; cycle++)
        {
            (await ui.PostAsync("/api/client/devices/1-1.4/detach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
            var expected = cycle + 1;
            await EventuallyAsync(async () =>
            {
                using var response = await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct);
                await Task.Delay(300, ct);
                return _clientKernel.Attaches == expected;
            }, ct);
        }

        await Task.Delay(TimeSpan.FromSeconds(1), ct);
        _clientKernel.DetachedPorts.Count.Should().Be(cycles);
        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/client/devices", ct);
        devices.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean().Should().BeTrue();
    }

    [Fact]
    public async Task Stopping_the_client_does_not_wait_for_its_relays_and_keeps_the_device_remembered()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        await AttachAsync(hub, ct);
        var usb = _client.Services.GetRequiredService<UsbClient>();

        var started = DateTime.UtcNow;
        await usb.StopAsync(ct);

        // A relay nobody stopped made the stop wait out its five seconds.
        (DateTime.UtcNow - started).Should().BeLessThan(TimeSpan.FromSeconds(3));
        _clientKernel.DetachedPorts.Should().Contain(8);
        File.ReadAllText(_client.ConfigFile("attached.json")).Should().Contain("1-1.4");
    }

    [Fact]
    public async Task A_detach_that_cannot_be_saved_is_a_fault_on_the_device_and_the_device_is_detached()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        await AttachAsync(hub, ct);
        var path = _client.ConfigFile("attached.json");
        // A folder where the file belongs: the save cannot replace it on Windows or Linux, and no permission or lock is involved, so it also holds for root in a container.
        File.Delete(path);
        Directory.CreateDirectory(path);
        try
        {
            using var ui = _client.CreateUiClient();
            (await ui.PostAsync("/api/client/devices/1-1.4/detach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

            _clientKernel.DetachedPorts.Should().Contain(8);
            var devices = await ui.GetFromJsonAsync<JsonElement>("/api/client/devices", ct);
            devices.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean().Should().BeFalse();
            devices.GetProperty("devices")[0].GetProperty("fault").GetString().Should().Contain("could not save");
        }
        finally
        {
            Directory.Delete(path);
        }
    }

    [Fact]
    public async Task Attaching_a_device_that_is_attached_or_on_its_way_does_nothing()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await LinkAsync(ct);
        var attached = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean());
        using var ui = _client.CreateUiClient();

        // Twice at once, while the first is still in flight; then again once it is attached.
        var first = ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct);
        var second = ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct);
        await Task.WhenAll(first, second);
        await Link.WithLastAsync(attached, hub, ct);
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        // The request has been answered, so an attach it started would be counted by now.
        _client.Services.GetRequiredService<UsbClient>().AttachStarts.Should().Be(1);
        _clientKernel.Attaches.Should().Be(1);
        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/client/devices", ct);
        devices.GetProperty("devices")[0].GetProperty("fault").ValueKind.Should().Be(JsonValueKind.Null);
    }

    [Fact]
    public async Task A_bus_id_that_is_not_one_is_refused_by_the_endpoint()
    {
        var ct = TestContext.Current.CancellationToken;
        using var ui = _client.CreateUiClient();

        using var response = await ui.PostAsync("/api/client/devices/1-1.4;calc/attach", null, ct);

        response.StatusCode.Should().Be(HttpStatusCode.BadRequest);
    }

    [Fact]
    public async Task Devices_with_a_bus_id_that_is_not_one_never_reach_the_client_list()
    {
        var ct = TestContext.Current.CancellationToken;
        _serverKernel.Devices = [FakeKernel.Dongle, FakeKernel.Dongle with { BusId = "1-1.5 --stop" }];

        // The list the link waits for has one device in it: the server sent two, and the bad one was dropped.
        await using var hub = await LinkAsync(ct);

        using var ui = _client.CreateUiClient();
        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/client/devices", ct);
        devices.GetProperty("devices").GetArrayLength().Should().Be(1);
        devices.GetProperty("devices")[0].GetProperty("device").GetProperty("busId").GetString().Should().Be("1-1.4");
    }

    [Fact]
    public void An_import_reply_for_another_bus_id_or_another_device_is_refused()
    {
        var picked = new DeviceView("1-1.4", "064f:2af9", "CodeMeter-Stick", null, "full", "shared", null, null, null);

        UsbClient.Check(FakeKernel.Dongle, "1-1.4", picked);

        var otherPort = () => UsbClient.Check(FakeKernel.Dongle with { BusId = "1-1.5" }, "1-1.4", picked);
        var otherDevice = () => UsbClient.Check(FakeKernel.Dongle with { VendorId = 0x0403, ProductId = 0x6001 }, "1-1.4", picked);
        otherPort.Should().Throw<InvalidOperationException>().WithMessage("*different device*");
        otherDevice.Should().Throw<InvalidOperationException>().WithMessage("*0403:6001*");
    }

    [Fact]
    public void Failures_are_described_for_a_person()
    {
        RemoteFailure.Describe(new AuthenticationException()).Should().Contain("fingerprint");
        RemoteFailure.Describe(new HttpRequestException("x", new AuthenticationException())).Should().Contain("fingerprint");
        RemoteFailure.Describe(new HttpRequestException("Connection refused")).Should().StartWith("Could not reach the server");
        RemoteFailure.Describe(new JsonException("bad")).Should().Contain("did not answer like a Koppel server");
        RemoteFailure.Describe(new TimeoutException("too slow")).Should().Be("too slow");
    }

    [Fact]
    public async Task A_gateway_port_that_is_taken_does_not_stop_the_host_and_shows_in_the_client_list()
    {
        var ct = TestContext.Current.CancellationToken;
        var blocker = new TcpListener(IPAddress.Loopback, 0);
        blocker.Start();
        var port = ((IPEndPoint)blocker.LocalEndpoint).Port;
        try
        {
            using var client = new KoppelFactory(kestrel: true, configuration: new Dictionary<string, string> { ["Gateway:Port"] = port.ToString(CultureInfo.InvariantCulture) });
            await using var hub = await client.ConnectUiHubAsync(ct);
            var faulted = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("gatewayFault").ValueKind == JsonValueKind.String);
            using var ui = client.CreateUiClient();

            // Only a client wants the gateway.
            (await ui.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).EnsureSuccessStatusCode();

            var pushed = await Link.WithLastAsync(faulted, hub, ct);
            pushed.GetProperty("gatewayFault").GetString().Should().Contain(port.ToString(CultureInfo.InvariantCulture));
            pushed.GetProperty("gateway").GetBoolean().Should().BeFalse();
            (await ui.GetAsync("/api/client/devices", ct)).StatusCode.Should().Be(HttpStatusCode.OK, "the host is still running");

            // Once the port is free the gateway binds on its own.
            var listening = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("gateway").GetBoolean());
            blocker.Stop();
            await Link.WithLastAsync(listening, hub, ct);

            // A server has no gateway.
            var stopped = Link.Waiter(hub, "ClientDevicesChanged", d => !d.GetProperty("gateway").GetBoolean());
            (await ui.PatchAsJsonAsync("/api/settings", new { role = "server" }, ct)).EnsureSuccessStatusCode();
            await Link.WithLastAsync(stopped, hub, ct);
        }
        finally
        {
            blocker.Stop();
        }
    }

    [Fact]
    public async Task Stopping_the_client_detaches_what_the_driver_holds()
    {
        var ct = TestContext.Current.CancellationToken;
        using var driver = new FakeUsbIpDriver();
        using var client = new KoppelFactory(
            kestrel: true,
            services: s => s.AddSingleton<IUsbIpDriver>(driver),
            configuration: new Dictionary<string, string> { ["Gateway:Port"] = KoppelFactory.FreePort().ToString(CultureInfo.InvariantCulture) });
        await using var hub = await client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        var attached = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("devices").EnumerateArray().Any(x => x.GetProperty("port").ValueKind == JsonValueKind.Number));
        await Link.ConnectAsync(_server, client, ct);
        await Link.WithLastAsync(linked, hub, ct);
        using var ui = client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        await Link.WithLastAsync(attached, hub, ct);

        await client.Services.GetRequiredService<UsbClient>().StopAsync(ct);

        driver.Detached.Should().Equal("1-1.4@1");
        File.ReadAllText(client.ConfigFile("attached.json")).Should().Contain("1-1.4", "stopping does not forget the device");
    }

    /// <summary>Links the client to the server and waits until it lists the dongle.</summary>
    private async Task<HubConnection> LinkAsync(CancellationToken ct)
    {
        var hub = await _client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        await Link.ConnectAsync(_server, _client, ct);
        await Link.WithLastAsync(linked, hub, ct);
        return hub;
    }

    private async Task AttachAsync(HubConnection hub, CancellationToken ct)
    {
        var attached = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean());
        using var ui = _client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        await Link.WithLastAsync(attached, hub, ct);
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
