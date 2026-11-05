using System.Globalization;
using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Core.Activity;
using Koppel.Core.Usb;
using Koppel.Usb;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// What Koppel does when its link followed the server to another address: the saved connection follows,
/// the activity log says so, and the attached device stays. Skakel's own tests cover how the link finds
/// the server and proves it before it moves; here the move is handed to the client as the link raises it.
/// Two real hosts on Kestrel with fake kernels.
/// </summary>
public sealed class ServerMoveTests : IDisposable
{
    private readonly FakeKernel _serverKernel = new();
    private readonly FakeKernel _clientKernel = new();
    private readonly KoppelFactory _server;
    private readonly KoppelFactory _client;

    public ServerMoveTests()
    {
        _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_serverKernel), shareDongle: true);
        _client = new KoppelFactory(
            kestrel: true,
            services: s => s.AddSingleton<IUsbAttacher>(_clientKernel),
            configuration: new Dictionary<string, string> { ["Gateway:Port"] = KoppelFactory.FreePort().ToString(CultureInfo.InvariantCulture) },
            role: "client");
    }

    public void Dispose()
    {
        _client.Dispose();
        _server.Dispose();
    }

    [Fact]
    public async Task A_move_the_link_followed_is_saved_and_logged_and_the_device_stays_attached()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await _client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        await Link.ConnectAsync(_server, _client, ct);
        await Link.WithLastAsync(linked, hub, ct);
        var attached = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean());
        using var ui = _client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        await Link.WithLastAsync(attached, hub, ct);
        var to = $"https://127.0.0.1:{_server.HttpsPort}";
        var moved = Link.Waiter(hub, "ConnectionChanged", c => c.GetProperty("serverUrl").GetString() == to);
        var back = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean());

        // The same server, saved at localhost, heard at 127.0.0.1.
        var usb = _client.Services.GetRequiredService<UsbClient>();
        await usb.OnLinkMovedAsync(usb.Remote!, new Uri($"{to}/"));

        var connection = await moved.WaitAsync(TimeSpan.FromSeconds(20), ct);
        connection.GetProperty("serverFingerprint").GetString().Should().Be(RemoteAccess.Fingerprint(_server), "only the address changes");
        connection.GetProperty("hasSecret").GetBoolean().Should().BeTrue();
        await Link.WithLastAsync(back, hub, ct);
        _clientKernel.DetachedPorts.Should().BeEmpty("the same server at another address is not a new connection");
        _client.Services.GetRequiredService<IActivityReader>().Recent(50)
            .Should().Contain(e => e.Reason == ActivityReason.ServerMoved && e.Summary == $"Server moved from localhost:{_server.HttpsPort} to 127.0.0.1:{_server.HttpsPort}");
    }

    [Fact]
    public async Task A_move_does_not_overwrite_a_connection_the_person_saved_meanwhile()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await _client.ConnectUiHubAsync(ct);
        var linked = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("link").GetString() == "connected");
        await Link.ConnectAsync(_server, _client, ct);
        await Link.WithLastAsync(linked, hub, ct);
        var usb = _client.Services.GetRequiredService<UsbClient>();
        var stale = usb.Remote!;
        using var ui = _client.CreateUiClient();
        var saved = await ui.GetFromJsonAsync<JsonElement>("/api/client/connection", ct);
        (await ui.PutAsJsonAsync("/api/client/connection", new
        {
            serverUrl = $"https://localhost:{KoppelFactory.FreePort()}",
            serverFingerprint = saved.GetProperty("serverFingerprint").GetString(),
            clientId = "someone-else",
            clientSecret = "their-secret",
        }, ct)).EnsureSuccessStatusCode();

        await usb.OnLinkMovedAsync(stale, new Uri($"https://127.0.0.1:{_server.HttpsPort}/"));

        (await ui.GetFromJsonAsync<JsonElement>("/api/client/connection", ct)).GetProperty("clientId").GetString().Should().Be("someone-else");
        _client.Services.GetRequiredService<IActivityReader>().Recent(50).Should().NotContain(e => e.Reason == ActivityReason.ServerMoved);
    }
}
