using System.Globalization;
using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Core.Usb;
using Microsoft.AspNetCore.SignalR.Client;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Usb;

/// <summary>
/// A server replacing its certificate while a client is paired: the client learns the next fingerprint
/// over its link, the server sees that it has it, and after the switch the client pins the new one and
/// keeps its device. Two real hosts on Kestrel with fake kernels, as in the end-to-end tests.
/// </summary>
public sealed class CertificateHandoverTests : IDisposable
{
    private readonly FakeKernel _serverKernel = new();
    private readonly FakeKernel _clientKernel = new();
    private readonly KoppelFactory _server;
    private readonly KoppelFactory _client;

    public CertificateHandoverTests()
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
    public async Task A_linked_client_learns_the_next_certificate_and_the_server_shows_it_has_it()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await LinkAsync(ct);
        await using var serverHub = await _server.ConnectUiHubAsync(ct);
        var received = Link.Waiter(serverHub, "ClientsChanged", clients => clients.EnumerateArray().Any(c => c.GetProperty("hasNextCertificate").GetBoolean()));

        var next = await PrepareAsync(ct);

        await received.WaitAsync(TimeSpan.FromSeconds(20), ct);
        (await ClientConnectionAsync(ct)).GetProperty("nextServerFingerprint").GetString().Should().Be(next);
    }

    [Fact]
    public async Task After_the_switch_the_client_pins_the_new_certificate_and_its_device_stays_attached()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await LinkAsync(ct);
        await AttachAsync(clientHub, ct);
        var learned = Link.Waiter(clientHub, "ConnectionChanged", c => c.GetProperty("nextServerFingerprint").ValueKind == JsonValueKind.String);
        var next = await PrepareAsync(ct);
        await learned.WaitAsync(TimeSpan.FromSeconds(20), ct);
        var pinned = Link.Waiter(clientHub, "ConnectionChanged", c => c.GetProperty("serverFingerprint").GetString() == next);

        using var server = _server.CreateUiClient();
        (await server.PostAsync("/api/server/keys/next-certificate/switch", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        var connection = await pinned.WaitAsync(TimeSpan.FromSeconds(20), ct);
        connection.GetProperty("nextServerFingerprint").ValueKind.Should().Be(JsonValueKind.Null);
        _clientKernel.DetachedPorts.Should().BeEmpty("the same server switching certificates is not a new connection");
        using var ui = _client.CreateUiClient();
        var devices = await ui.GetFromJsonAsync<JsonElement>("/api/client/devices", ct);
        devices.GetProperty("devices")[0].GetProperty("attachedHere").GetBoolean().Should().BeTrue();
        // The remembered device moves to the new fingerprint, so it attaches again after a restart.
        await EventuallyAsync(() => Task.FromResult(File.ReadAllText(_client.ConfigFile("attached.json")).Contains(next!, StringComparison.Ordinal)), ct);

        // New connections now see the new certificate, and the client trusts it.
        using var test = await ui.PostAsync("/api/client/connection/test", null, ct);
        (await test.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("outcome").GetString().Should().Be("ok");
    }

    [Fact]
    public async Task A_client_that_links_during_a_handover_is_told_the_next_certificate_too()
    {
        var ct = TestContext.Current.CancellationToken;
        var next = await PrepareAsync(ct);

        await using var clientHub = await LinkAsync(ct);

        await EventuallyAsync(async () => (await ClientConnectionAsync(ct)).GetProperty("nextServerFingerprint").GetString() == next, ct);
    }

    [Fact]
    public async Task Cancelling_the_next_certificate_makes_the_client_forget_it()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await LinkAsync(ct);
        var learned = Link.Waiter(clientHub, "ConnectionChanged", c => c.GetProperty("nextServerFingerprint").ValueKind == JsonValueKind.String);
        await PrepareAsync(ct);
        await learned.WaitAsync(TimeSpan.FromSeconds(20), ct);
        var forgot = Link.Waiter(clientHub, "ConnectionChanged", c => c.GetProperty("nextServerFingerprint").ValueKind == JsonValueKind.Null);

        using var server = _server.CreateUiClient();
        (await server.DeleteAsync("/api/server/keys/next-certificate", ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await forgot.WaitAsync(TimeSpan.FromSeconds(20), ct);
    }

    [Fact]
    public async Task A_client_that_missed_the_whole_handover_reports_a_certificate_mismatch()
    {
        var ct = TestContext.Current.CancellationToken;
        using var server = _server.CreateUiClient();
        var old = (await server.GetFromJsonAsync<JsonElement>("/api/server/keys", ct)).GetProperty("fingerprint").GetString();
        await PrepareAsync(ct);
        (await server.PostAsync("/api/server/keys/next-certificate/switch", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        // The client was off the whole time and still pins the old certificate.
        using var created = await server.PostAsJsonAsync("/api/server/clients", new { name = "offline-client" }, ct);
        var credentials = await created.Content.ReadFromJsonAsync<JsonElement>(ct);
        using var ui = _client.CreateUiClient();
        (await ui.PutAsJsonAsync("/api/client/connection", new
        {
            serverUrl = $"https://localhost:{_server.HttpsPort}",
            serverFingerprint = old,
            clientId = credentials.GetProperty("clientId").GetString(),
            clientSecret = credentials.GetProperty("clientSecret").GetString(),
        }, ct)).EnsureSuccessStatusCode();

        using var test = await ui.PostAsync("/api/client/connection/test", null, ct);
        (await test.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("outcome").GetString().Should().Be("certificateMismatch");
    }

    /// <summary>Prepares the next certificate on the server and returns its fingerprint.</summary>
    private async Task<string?> PrepareAsync(CancellationToken ct)
    {
        using var server = _server.CreateUiClient();
        (await server.PostAsync("/api/server/keys/next-certificate", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        return (await server.GetFromJsonAsync<JsonElement>("/api/server/keys", ct)).GetProperty("nextFingerprint").GetString();
    }

    private async Task<JsonElement> ClientConnectionAsync(CancellationToken ct)
    {
        using var ui = _client.CreateUiClient();
        return await ui.GetFromJsonAsync<JsonElement>("/api/client/connection", ct);
    }

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
