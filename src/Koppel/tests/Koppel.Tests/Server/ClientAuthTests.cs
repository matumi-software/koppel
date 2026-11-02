using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.SignalR.Client;

namespace Koppel.Tests.Server;

/// <summary>The server as an OAuth server: registered clients get tokens, and only they reach the remote API.</summary>
public sealed class ClientAuthTests : IDisposable
{
    private readonly KoppelFactory _koppel = new();

    public void Dispose() => _koppel.Dispose();

    [Fact]
    public async Task Created_client_gets_a_token_and_reaches_the_remote_api_over_https()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await CreateClientAsync("bench-2", ct);
        using var https = _koppel.CreateHttpsClient();

        var token = await RequestTokenAsync(https, clientId, secret, ct);
        token.StatusCode.Should().Be(HttpStatusCode.OK);
        var accessToken = (await token.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("access_token").GetString();

        using var hello = await GetHelloAsync(https, accessToken!, ct);
        hello.StatusCode.Should().Be(HttpStatusCode.OK);
        (await hello.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("clientId").GetString().Should().Be(clientId);
    }

    [Fact]
    public async Task Wrong_secret_is_refused_as_invalid_client()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, _) = await CreateClientAsync("bench-2", ct);
        using var https = _koppel.CreateHttpsClient();

        using var token = await RequestTokenAsync(https, clientId, "not-the-secret", ct);

        // RFC 6749 5.2: invalid_client may answer 401, which OpenIddict does.
        token.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        (await token.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("error").GetString().Should().Be("invalid_client");
    }

    [Fact]
    public async Task Token_requests_over_plain_http_are_refused()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await CreateClientAsync("bench-2", ct);
        using var http = _koppel.CreateUiClient();

        using var token = await RequestTokenAsync(http, clientId, secret, ct);

        token.StatusCode.Should().Be(HttpStatusCode.BadRequest);
    }

    [Fact]
    public async Task Revoked_client_is_refused_even_with_a_valid_token()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await CreateClientAsync("bench-2", ct);
        using var https = _koppel.CreateHttpsClient();
        using var token = await RequestTokenAsync(https, clientId, secret, ct);
        var accessToken = (await token.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("access_token").GetString()!;

        using var ui = _koppel.CreateUiClient();
        using var revoke = await ui.DeleteAsync($"/api/server/clients/{clientId}", ct);
        revoke.StatusCode.Should().Be(HttpStatusCode.Accepted);

        using var hello = await GetHelloAsync(https, accessToken, ct);
        hello.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        using var again = await RequestTokenAsync(https, clientId, secret, ct);
        again.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
    }

    [Fact]
    public async Task Creating_a_client_is_confirmed_over_SignalR_and_the_file_is_encrypted()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await _koppel.ConnectUiHubAsync(ct);
        var pushed = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        hub.On<JsonElement>("ClientsChanged", clients => pushed.TrySetResult(clients));

        var (clientId, secret) = await CreateClientAsync("a-very-recognisable-name", ct);

        var clients = await pushed.Task.WaitAsync(TimeSpan.FromSeconds(5), ct);
        clients.EnumerateArray().Should().ContainSingle(c => c.GetProperty("id").GetString() == clientId);
        clients.GetRawText().Should().NotContain(secret);
        var onDisk = await File.ReadAllTextAsync(_koppel.ConfigFile("secrets.json"), ct);
        onDisk.Should().NotContain("a-very-recognisable-name").And.NotContain(clientId);
    }

    [Fact]
    public async Task Secret_can_be_fetched_again_and_rotating_it_retires_the_old_one()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, secret) = await CreateClientAsync("bench-2", ct);
        using var ui = _koppel.CreateUiClient();

        var fetched = await ui.GetFromJsonAsync<JsonElement>($"/api/server/clients/{clientId}/secret", ct);
        fetched.GetProperty("clientSecret").GetString().Should().Be(secret);

        using var rotate = await ui.PostAsync($"/api/server/clients/{clientId}/rotate", null, ct);
        rotate.StatusCode.Should().Be(HttpStatusCode.OK);
        var rotated = (await rotate.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("clientSecret").GetString()!;
        rotated.Should().NotBe(secret);

        using var https = _koppel.CreateHttpsClient();
        using var oldToken = await RequestTokenAsync(https, clientId, secret, ct);
        oldToken.StatusCode.Should().Be(HttpStatusCode.Unauthorized);
        using var newToken = await RequestTokenAsync(https, clientId, rotated, ct);
        newToken.StatusCode.Should().Be(HttpStatusCode.OK);
    }

    [Fact]
    public async Task Adding_and_revoking_a_client_is_in_the_activity_log()
    {
        var ct = TestContext.Current.CancellationToken;
        var (clientId, _) = await CreateClientAsync("bench-2", ct);
        using var ui = _koppel.CreateUiClient();
        using var revoke = await ui.DeleteAsync($"/api/server/clients/{clientId}", ct);

        var activity = await ui.GetFromJsonAsync<JsonElement>($"/api/activity?entity={clientId}", ct);

        activity.EnumerateArray().Select(e => e.GetProperty("reason").GetString()).Should().Equal("clientRevoked", "clientAdded");
        activity[0].GetProperty("summary").GetString().Should().Be("Client bench-2 was revoked");
    }

    private async Task<(string ClientId, string Secret)> CreateClientAsync(string name, CancellationToken ct)
    {
        using var ui = _koppel.CreateUiClient();
        using var response = await ui.PostAsJsonAsync("/api/server/clients", new { name }, ct);
        response.StatusCode.Should().Be(HttpStatusCode.Created);
        var body = await response.Content.ReadFromJsonAsync<JsonElement>(ct);
        return (body.GetProperty("clientId").GetString()!, body.GetProperty("clientSecret").GetString()!);
    }

    private static Task<HttpResponseMessage> RequestTokenAsync(HttpClient http, string clientId, string secret, CancellationToken ct) =>
        http.PostAsync("/connect/token", new FormUrlEncodedContent(new Dictionary<string, string>
        {
            ["grant_type"] = "client_credentials",
            ["client_id"] = clientId,
            ["client_secret"] = secret,
        }), ct);

    private static Task<HttpResponseMessage> GetHelloAsync(HttpClient http, string accessToken, CancellationToken ct)
    {
        var request = new HttpRequestMessage(HttpMethod.Get, "/api/remote/hello");
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", accessToken);
        return http.SendAsync(request, ct);
    }
}
