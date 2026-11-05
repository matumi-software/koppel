using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text;
using System.Text.Json;
using Microsoft.Extensions.DependencyInjection;
using Skakel.Server;

namespace Koppel.Tests.Server;

/// <summary>
/// Rotating the token signing key and preparing, cancelling and switching the certificate through the
/// endpoints the UI calls. Skakel keeps the keys; its own tests cover the rotation underneath.
/// </summary>
public sealed class ServerKeysEndpointTests : IDisposable
{
    private readonly KoppelFactory _koppel = new();

    public void Dispose() => _koppel.Dispose();

    [Fact]
    public async Task A_token_signed_before_a_rotation_still_works_and_new_tokens_carry_the_new_key()
    {
        var ct = TestContext.Current.CancellationToken;
        using var ui = _koppel.CreateUiClient();
        using var created = await ui.PostAsJsonAsync("/api/server/clients", new { name = "bench-1" }, ct);
        var client = await created.Content.ReadFromJsonAsync<JsonElement>(ct);
        var (id, secret) = (client.GetProperty("clientId").GetString()!, client.GetProperty("clientSecret").GetString()!);
        var first = (await ui.GetFromJsonAsync<JsonElement>("/api/server/keys", ct)).GetProperty("signingKeyId").GetString();
        var before = await TokenAsync(id, secret, ct);

        using var rotate = await ui.PostAsync("/api/server/keys/signing-key/rotate", null, ct);

        rotate.StatusCode.Should().Be(HttpStatusCode.Accepted);
        var after = await TokenAsync(id, secret, ct);
        KeyIdOf(before).Should().Be(first);
        KeyIdOf(after).Should().NotBe(first).And.Be(_koppel.Services.GetRequiredService<SkakelKeys>().View.SigningKeyId);
        (await HelloAsync(before, ct)).Should().Be(HttpStatusCode.OK, "the old key is still accepted until its tokens expire");
        (await HelloAsync(after, ct)).Should().Be(HttpStatusCode.OK);
    }

    [Fact]
    public async Task Prepare_cancel_and_switch_are_confirmed_over_the_hub_and_refused_out_of_turn()
    {
        var ct = TestContext.Current.CancellationToken;
        await using var hub = await _koppel.ConnectUiHubAsync(ct);
        using var ui = _koppel.CreateUiClient();
        var current = (await ui.GetFromJsonAsync<JsonElement>("/api/server/keys", ct)).GetProperty("fingerprint").GetString();

        (await ui.PostAsync("/api/server/keys/next-certificate/switch", null, ct)).StatusCode.Should().Be(HttpStatusCode.BadRequest);
        (await ui.DeleteAsync("/api/server/keys/next-certificate", ct)).StatusCode.Should().Be(HttpStatusCode.BadRequest);

        var prepared = Usb.Link.Waiter(hub, "ServerKeysChanged", k => k.GetProperty("nextFingerprint").ValueKind == JsonValueKind.String);
        (await ui.PostAsync("/api/server/keys/next-certificate", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        var next = (await prepared.WaitAsync(TimeSpan.FromSeconds(10), ct)).GetProperty("nextFingerprint").GetString();
        (await ui.PostAsync("/api/server/keys/next-certificate", null, ct)).StatusCode.Should().Be(HttpStatusCode.BadRequest, "one is already prepared");

        var switched = Usb.Link.Waiter(hub, "ServerKeysChanged", k => k.GetProperty("fingerprint").GetString() == next);
        (await ui.PostAsync("/api/server/keys/next-certificate/switch", null, ct)).StatusCode.Should().Be(HttpStatusCode.Accepted);
        var keys = await switched.WaitAsync(TimeSpan.FromSeconds(10), ct);

        keys.GetProperty("nextFingerprint").ValueKind.Should().Be(JsonValueKind.Null);
        next.Should().NotBe(current);
        var activity = await ui.GetFromJsonAsync<JsonElement>("/api/activity?max=200", ct);
        activity.EnumerateArray().Select(a => a.GetProperty("reason").GetString()).Should().Contain(["certificatePrepared", "certificateSwitched"]);
    }

    private async Task<string> TokenAsync(string clientId, string secret, CancellationToken ct)
    {
        using var https = _koppel.CreateHttpsClient();
        using var response = await https.PostAsync("/skakel/v1/token", new FormUrlEncodedContent(new Dictionary<string, string>
        {
            ["grant_type"] = "client_credentials",
            ["client_id"] = clientId,
            ["client_secret"] = secret,
        }), ct);
        response.EnsureSuccessStatusCode();
        return (await response.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("access_token").GetString()!;
    }

    private async Task<HttpStatusCode> HelloAsync(string token, CancellationToken ct)
    {
        using var https = _koppel.CreateHttpsClient();
        using var request = new HttpRequestMessage(HttpMethod.Get, "/skakel/v1/hello");
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
        using var response = await https.SendAsync(request, ct);
        return response.StatusCode;
    }

    private static string? KeyIdOf(string token)
    {
        var header = token.Split('.')[0].Replace('-', '+').Replace('_', '/');
        header = header.PadRight(header.Length + (4 - header.Length % 4) % 4, '=');
        return JsonDocument.Parse(Encoding.UTF8.GetString(Convert.FromBase64String(header))).RootElement.GetProperty("kid").GetString();
    }
}
