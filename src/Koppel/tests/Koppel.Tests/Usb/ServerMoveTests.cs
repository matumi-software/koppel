using System.Globalization;
using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Core.Activity;
using Koppel.Core.Discovery;
using Koppel.Core.Usb;
using Koppel.Discovery;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// A client whose server stopped answering at the saved address, and which hears it somewhere else on the
/// network, follows it there, keeping its devices. Only a server that is gone from its address is
/// followed, and only to one with the pinned certificate. Two real hosts on Kestrel with fake kernels, and
/// a stand-in for what discovery hears.
/// </summary>
public sealed class ServerMoveTests : IDisposable
{
    private readonly FakeKernel _serverKernel = new();
    private readonly FakeKernel _clientKernel = new();
    private readonly FakeDiscovery _discovery = new();
    private readonly KoppelFactory _server;
    private readonly KoppelFactory _client;

    public ServerMoveTests()
    {
        _server = new KoppelFactory(kestrel: true, services: s => s.AddSingleton<IUsbHost>(_serverKernel), shareDongle: true);
        _client = new KoppelFactory(
            kestrel: true,
            services: s =>
            {
                s.AddSingleton<IUsbAttacher>(_clientKernel);
                s.AddSingleton<IDiscoveredServers>(_discovery);
            },
            configuration: new Dictionary<string, string> { ["Gateway:Port"] = KoppelFactory.FreePort().ToString(CultureInfo.InvariantCulture) },
            role: "client");
    }

    public void Dispose()
    {
        _client.Dispose();
        _server.Dispose();
    }

    /// <summary>Where the server really is.</summary>
    private string ServerUrl => $"https://127.0.0.1:{_server.HttpsPort}";

    private string ServerFingerprint => RemoteAccess.Fingerprint(_server);

    /// <summary>An address nobody listens on, as the old port is after the server moved.</summary>
    private static string Gone() => $"https://127.0.0.1:{KoppelFactory.FreePort()}";

    [Fact]
    public async Task A_client_follows_its_server_to_the_address_discovery_hears_it_on()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await _client.ConnectUiHubAsync(ct);
        var failing = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("link").GetString() == "error");
        var linked = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        var gone = Gone();
        var credentials = await RemoteAccess.CreateClientAsync(_server, "moving-client", ct);
        await SaveAsync(gone, ServerFingerprint, credentials, ct);
        await Link.WithLastAsync(failing, hub, ct);
        var moved = Link.Waiter(hub, "ConnectionChanged", c => c.GetProperty("serverUrl").GetString() == ServerUrl);

        await _discovery.PublishAsync(Heard(_server.HttpsPort, ServerFingerprint));

        var connection = await moved.WaitAsync(TimeSpan.FromSeconds(20), ct);
        connection.GetProperty("serverFingerprint").GetString().Should().Be(ServerFingerprint, "only the address changes");
        connection.GetProperty("clientId").GetString().Should().Be(credentials.ClientId);
        connection.GetProperty("hasSecret").GetBoolean().Should().BeTrue();
        await Link.WithLastAsync(linked, hub, ct);
        _client.Services.GetRequiredService<IActivityReader>().Recent(50)
            .Should().Contain(e => e.Reason == ActivityReason.ServerMoved && e.Summary == $"Server moved from {new Uri(gone).Authority} to 127.0.0.1:{_server.HttpsPort}");
    }

    [Fact]
    public async Task A_client_keeps_its_attached_device_when_its_server_moves()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await _client.ConnectUiHubAsync(ct);
        var credentials = await RemoteAccess.CreateClientAsync(_server, "moving-client", ct);
        var linked = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices").GetArrayLength() == 1);
        await SaveAsync(ServerUrl, ServerFingerprint, credentials, ct);
        await Link.WithLastAsync(linked, hub, ct);
        var attached = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean());
        using var ui = _client.CreateUiClient();
        (await ui.PostAsync("/api/client/devices/1-1.4/attach", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        await Link.WithLastAsync(attached, hub, ct);

        // The same server saved at an address where it does not answer: the device stays, the link fails there.
        var failing = Link.Waiter(hub, "ClientDevicesChanged", d => d.GetProperty("link").GetString() == "error");
        await SaveAsync(Gone(), ServerFingerprint, credentials, ct);
        await Link.WithLastAsync(failing, hub, ct);
        var back = Link.Waiter(hub, "ClientDevicesChanged", d =>
            d.GetProperty("link").GetString() == "connected" && d.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean());

        await _discovery.PublishAsync(Heard(_server.HttpsPort, ServerFingerprint));

        await Link.WithLastAsync(back, hub, ct);
        _clientKernel.DetachedPorts.Should().BeEmpty("the same server at another address is not a new connection");
        (await ui.GetFromJsonAsync<JsonElement>("/api/client/connection", ct)).GetProperty("serverUrl").GetString().Should().Be(ServerUrl);
    }

    [Fact]
    public async Task A_client_does_not_follow_a_server_with_another_fingerprint()
    {
        var ct = TestContext.Current.CancellationToken;
        var gone = Gone();
        await _discovery.PublishAsync(Heard(_server.HttpsPort, ServerFingerprint));

        // Saved with a fingerprint nobody on the network has; the real server is heard, but it is another server.
        await StaysAsync(gone, Unknown, await RemoteAccess.CreateClientAsync(_server, "moving-client", ct), ct);
    }

    [Fact]
    public async Task A_client_does_not_move_on_a_certificate_mismatch()
    {
        var ct = TestContext.Current.CancellationToken;
        // The server answers at the saved address with a certificate the client does not pin. Something with
        // the pinned fingerprint is heard elsewhere, but the server did answer, so it has not moved.
        await _discovery.PublishAsync(Heard(KoppelFactory.FreePort(), Unknown));

        await StaysAsync(ServerUrl, Unknown, await RemoteAccess.CreateClientAsync(_server, "moving-client", ct), ct);
    }

    [Fact]
    public async Task A_client_does_not_move_when_the_server_refuses_its_credentials()
    {
        var ct = TestContext.Current.CancellationToken;
        await _discovery.PublishAsync(Heard(KoppelFactory.FreePort(), ServerFingerprint));

        await StaysAsync(ServerUrl, ServerFingerprint, ("client-unknown", "not-the-secret"), ct);
    }

    private const string Unknown = "AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA";

    private static DiscoveredServer Heard(int port, string fingerprint) => new("test-server", "127.0.0.1", port, fingerprint, "0.1.0");

    /// <summary>
    /// Saves the connection and waits for the link to fail twice. The second failure comes after the first
    /// one's look at discovery finished, so by then a move would have happened. Then checks none did.
    /// </summary>
    private async Task StaysAsync(string url, string fingerprint, (string ClientId, string Secret) credentials, CancellationToken ct)
    {
        await using var hub = await _client.ConnectUiHubAsync(ct);
        // Pushes for one hub arrive one at a time, so the count needs no lock.
        string? last = null;
        var failures = 0;
        var failedTwice = Link.Waiter(hub, "ClientDevicesChanged", d =>
        {
            var link = d.GetProperty("link").GetString();
            var failed = link == "error" && last != "error";
            last = link;
            return failed && ++failures == 2;
        });
        await SaveAsync(url, fingerprint, credentials, ct);

        await Link.WithLastAsync(failedTwice, hub, ct);

        using var ui = _client.CreateUiClient();
        (await ui.GetFromJsonAsync<JsonElement>("/api/client/connection", ct)).GetProperty("serverUrl").GetString().Should().Be(url);
        _client.Services.GetRequiredService<IActivityReader>().Recent(50).Should().NotContain(e => e.Reason == ActivityReason.ServerMoved);
    }

    private async Task SaveAsync(string url, string fingerprint, (string ClientId, string Secret) credentials, CancellationToken ct)
    {
        using var ui = _client.CreateUiClient();
        (await ui.PutAsJsonAsync("/api/client/connection", new
        {
            serverUrl = url,
            serverFingerprint = fingerprint,
            clientId = credentials.ClientId,
            clientSecret = credentials.Secret,
        }, ct)).EnsureSuccessStatusCode();
    }

    /// <summary>Stands in for what discovery hears on the network.</summary>
    private sealed class FakeDiscovery : IDiscoveredServers
    {
        public event Func<IReadOnlyList<DiscoveredServer>, Task>? Changed;

        public IReadOnlyList<DiscoveredServer> Servers { get; private set; } = [];

        /// <summary>Hears these servers, and waits for whoever listens, as the real browser does.</summary>
        public Task PublishAsync(params DiscoveredServer[] servers)
        {
            Servers = servers;
            return Changed?.Invoke(servers) ?? Task.CompletedTask;
        }
    }
}
