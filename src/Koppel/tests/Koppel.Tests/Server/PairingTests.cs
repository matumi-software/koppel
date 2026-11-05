using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Tests.Usb;
using Skakel.Protocol;

namespace Koppel.Tests.Server;

/// <summary>
/// Two real Koppel hosts pairing the way a person does it: the client asks, shows a code, and the
/// code is typed on the server. Covers Koppel's screens on both ends, wrong codes and the confirm rule;
/// Skakel's own tests cover the protocol underneath.
/// </summary>
public sealed class PairingTests : IDisposable
{
    private readonly KoppelFactory _server = new(kestrel: true);
    private readonly KoppelFactory _client = new(kestrel: true, role: "client");

    public void Dispose()
    {
        _client.Dispose();
        _server.Dispose();
    }

    [Fact]
    public async Task Typing_the_clients_code_on_the_server_pairs_and_links_them()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await _client.ConnectUiHubAsync(ct);
        await using var serverHub = await _server.ConnectUiHubAsync(ct);
        var waiting = Link.Waiter(clientHub, "PairingStatusChanged", s => s.GetProperty("state").GetString() == "waiting");
        var paired = Link.Waiter(clientHub, "PairingStatusChanged", s => s.GetProperty("state").GetString() == "paired");
        var connected = Link.Waiter(clientHub, "ConnectionChanged", c => c.GetProperty("hasSecret").GetBoolean());
        var linked = Link.Waiter(clientHub, "ClientDevicesChanged", d => d.GetProperty("link").GetString() == "connected");
        var requests = Link.Waiter(serverHub, "PairingRequestsChanged", r => r.GetProperty("requests").GetArrayLength() == 1);
        var cleared = Link.Waiter(serverHub, "PairingRequestsChanged", r => r.GetProperty("requests").GetArrayLength() == 0);
        var clients = Link.Waiter(serverHub, "ClientsChanged", c => c.GetArrayLength() == 1);

        using var clientUi = _client.CreateUiClient();
        (await clientUi.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).EnsureSuccessStatusCode();
        await StartPairingAsync(clientUi, ct);

        var code = (await waiting.WaitAsync(TimeSpan.FromSeconds(20), ct)).GetProperty("code").GetString()!;
        code.Should().MatchRegex("^[0-9]{6}$");
        var request = (await requests.WaitAsync(TimeSpan.FromSeconds(10), ct)).GetProperty("requests")[0];
        request.GetProperty("clientName").GetString().Should().Be(Environment.MachineName);
        request.TryGetProperty("code", out _).Should().BeFalse("the server never shows the code; a person types it");

        using var serverUi = _server.CreateUiClient();
        var id = request.GetProperty("id").GetString();
        var wrong = Wrong(code);
        using (var refused = await serverUi.PostAsJsonAsync($"/api/server/pairing/{id}/approve", new { code = wrong }, ct))
        {
            refused.StatusCode.Should().Be(HttpStatusCode.BadRequest);
        }

        using (var approved = await serverUi.PostAsJsonAsync($"/api/server/pairing/{id}/approve", new { code = $"{code[..3]} {code[3..]}" }, ct))
        {
            approved.StatusCode.Should().Be(HttpStatusCode.Accepted);
        }

        await cleared.WaitAsync(TimeSpan.FromSeconds(10), ct);
        await clients.WaitAsync(TimeSpan.FromSeconds(10), ct);
        await connected.WaitAsync(TimeSpan.FromSeconds(20), ct);
        await paired.WaitAsync(TimeSpan.FromSeconds(20), ct);
        await linked.WaitAsync(TimeSpan.FromSeconds(20), ct);
    }

    [Fact]
    public async Task Three_wrong_codes_refuse_the_request_and_the_client_hears_it()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var clientHub = await _client.ConnectUiHubAsync(ct);
        await using var serverHub = await _server.ConnectUiHubAsync(ct);
        var waiting = Link.Waiter(clientHub, "PairingStatusChanged", s => s.GetProperty("state").GetString() == "waiting");
        var denied = Link.Waiter(clientHub, "PairingStatusChanged", s => s.GetProperty("state").GetString() == "denied");
        var requests = Link.Waiter(serverHub, "PairingRequestsChanged", r => r.GetProperty("requests").GetArrayLength() == 1);

        using var clientUi = _client.CreateUiClient();
        await StartPairingAsync(clientUi, ct);
        var code = (await waiting.WaitAsync(TimeSpan.FromSeconds(20), ct)).GetProperty("code").GetString()!;
        var id = (await requests.WaitAsync(TimeSpan.FromSeconds(10), ct)).GetProperty("requests")[0].GetProperty("id").GetString();

        using var serverUi = _server.CreateUiClient();
        for (var i = 0; i < 3; i++)
        {
            using var refused = await serverUi.PostAsJsonAsync($"/api/server/pairing/{id}/approve", new { code = Wrong(code) }, ct);
        }

        using (var tooLate = await serverUi.PostAsJsonAsync($"/api/server/pairing/{id}/approve", new { code }, ct))
        {
            tooLate.StatusCode.Should().Be(HttpStatusCode.NotFound);
        }

        await denied.WaitAsync(TimeSpan.FromSeconds(20), ct);
        (await serverUi.GetFromJsonAsync<JsonElement>("/api/server/clients", ct)).GetArrayLength().Should().Be(0);
    }

    [Fact]
    public async Task Cancelling_on_the_client_withdraws_the_request_from_the_server()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var serverHub = await _server.ConnectUiHubAsync(ct);
        var requests = Link.Waiter(serverHub, "PairingRequestsChanged", r => r.GetProperty("requests").GetArrayLength() == 1);
        var withdrawn = Link.Waiter(serverHub, "PairingRequestsChanged", r => r.GetProperty("requests").GetArrayLength() == 0);

        using var clientUi = _client.CreateUiClient();
        await StartPairingAsync(clientUi, ct);
        await requests.WaitAsync(TimeSpan.FromSeconds(20), ct);
        (await clientUi.DeleteAsync("/api/client/pairing", ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);

        await withdrawn.WaitAsync(TimeSpan.FromSeconds(10), ct);
    }

    [Fact]
    public async Task Pairing_is_only_offered_over_https()
    {
        var ct = TestContext.Current.CancellationToken;
        using var plain = _server.CreateUiClient();

        using var response = await plain.PostAsJsonAsync("/skakel/v1/pair", new { clientName = "x", commitment = PairingCode.Commit("n") }, ct);

        response.StatusCode.Should().Be(HttpStatusCode.NotFound);
    }

    private async Task StartPairingAsync(HttpClient clientUi, CancellationToken ct)
    {
        using var started = await clientUi.PostAsJsonAsync("/api/client/pairing", new
        {
            serverUrl = $"https://localhost:{_server.HttpsPort}",
            serverFingerprint = await FingerprintAsync(ct),
            serverName = "test-server",
        }, ct);
        started.StatusCode.Should().Be(HttpStatusCode.Accepted);
    }

    private async Task<string> FingerprintAsync(CancellationToken ct)
    {
        using var serverUi = _server.CreateUiClient();
        return (await serverUi.GetFromJsonAsync<JsonElement>("/api/server/identity", ct)).GetProperty("fingerprint").GetString()!;
    }

    private static string Wrong(string code) => code == "000000" ? "111111" : "000000";
}
