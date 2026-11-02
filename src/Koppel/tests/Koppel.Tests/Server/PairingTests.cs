using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Koppel.Core.Security;
using Koppel.Tests.Usb;
using Microsoft.AspNetCore.SignalR.Client;

namespace Koppel.Tests.Server;

/// <summary>
/// Two real Koppel hosts pairing the way a person does it: the client asks, shows a code, and the
/// code is typed on the server. Covers the commitment protocol, wrong codes and the confirm rule.
/// </summary>
public sealed class PairingTests : IDisposable
{
    private readonly KoppelFactory _server = new(kestrel: true);
    private readonly KoppelFactory _client = new(kestrel: true);

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
        var requests = Link.Waiter(serverHub, "PairingRequestsChanged", r => r.GetArrayLength() == 1);
        var cleared = Link.Waiter(serverHub, "PairingRequestsChanged", r => r.GetArrayLength() == 0);
        var clients = Link.Waiter(serverHub, "ClientsChanged", c => c.GetArrayLength() == 1);

        using var clientUi = _client.CreateUiClient();
        (await clientUi.PatchAsJsonAsync("/api/settings", new { role = "client" }, ct)).EnsureSuccessStatusCode();
        await StartPairingAsync(clientUi, ct);

        var code = (await waiting.WaitAsync(TimeSpan.FromSeconds(20), ct)).GetProperty("code").GetString()!;
        code.Should().MatchRegex("^[0-9]{6}$");
        var request = (await requests.WaitAsync(TimeSpan.FromSeconds(10), ct))[0];
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
        var requests = Link.Waiter(serverHub, "PairingRequestsChanged", r => r.GetArrayLength() == 1);

        using var clientUi = _client.CreateUiClient();
        await StartPairingAsync(clientUi, ct);
        var code = (await waiting.WaitAsync(TimeSpan.FromSeconds(20), ct)).GetProperty("code").GetString()!;
        var id = (await requests.WaitAsync(TimeSpan.FromSeconds(10), ct))[0].GetProperty("id").GetString();

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
    public async Task A_nonce_that_does_not_match_its_commitment_is_refused()
    {
        var ct = TestContext.Current.CancellationToken;
        using var https = PinnedClient(await FingerprintAsync(ct));
        using var opened = await https.PostAsJsonAsync("/api/pair", new { clientName = "mallory", commitment = PairingCode.Commit("one nonce") }, ct);
        var requestId = (await opened.EnsureSuccessStatusCode().Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("requestId").GetString();

        using var revealed = await https.PostAsJsonAsync($"/api/pair/{requestId}/reveal", new { clientNonce = "another nonce" }, ct);

        revealed.StatusCode.Should().Be(HttpStatusCode.NotFound);
        using var serverUi = _server.CreateUiClient();
        (await serverUi.GetFromJsonAsync<JsonElement>("/api/server/pairing", ct)).GetArrayLength().Should().Be(0);
    }

    [Fact]
    public async Task Pairing_is_only_offered_over_https()
    {
        var ct = TestContext.Current.CancellationToken;
        using var plain = _server.CreateUiClient();

        using var response = await plain.PostAsJsonAsync("/api/pair", new { clientName = "x", commitment = PairingCode.Commit("n") }, ct);

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

    private HttpClient PinnedClient(string fingerprint) =>
        new(ServerCertificate.CreatePinnedHandler(fingerprint)) { BaseAddress = new Uri($"https://localhost:{_server.HttpsPort}") };

    private static string Wrong(string code) => code == "000000" ? "111111" : "000000";
}
