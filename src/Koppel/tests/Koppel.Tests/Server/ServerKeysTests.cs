using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text;
using System.Text.Json;
using Koppel.Core.Security;
using Koppel.Server;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging.Abstractions;

namespace Koppel.Tests.Server;

/// <summary>
/// Rotating the token signing key and preparing, cancelling and switching the certificate, on the
/// store against a manual clock and through the server's endpoints.
/// </summary>
public sealed class ServerKeysTests : IAsyncLifetime
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "koppel-tests", Guid.NewGuid().ToString("N"));
    private readonly ManualTimeProvider _time = new();
    private SecretBox _box = null!;
    private ServerSecretsStore _secrets = null!;

    public async ValueTask InitializeAsync()
    {
        _box = new SecretBox(Path.Combine(_directory, "keys"), "test");
        _secrets = await StartStoreAsync();
    }

    public ValueTask DisposeAsync()
    {
        _secrets.Dispose();
        _box.Dispose();
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }

        return ValueTask.CompletedTask;
    }

    [Fact]
    public async Task RotateSigningKeyAsync_signs_with_a_new_key_and_still_accepts_the_old_one()
    {
        var ct = TestContext.Current.CancellationToken;
        _secrets.SigningKey.KeyId.Should().Be("koppel-1");

        var id = await _secrets.RotateSigningKeyAsync(ct);

        _secrets.SigningKey.KeyId.Should().Be(id).And.NotBe("koppel-1");
        _secrets.ValidationKeys.Select(k => k.KeyId).Should().Equal(id, "koppel-1");
        _secrets.KeysView.SigningKeyId.Should().Be(id);
        _secrets.KeysView.RetiredKeysUntil.Should().Be(_time.GetUtcNow() + Koppel.Core.Auth.KoppelAuthServer.AccessTokenLifetime + ServerSecretsStore.RetiredKeyMargin);
    }

    [Fact]
    public async Task RotateSigningKeyAsync_tells_the_token_server_to_take_the_new_keys()
    {
        var ct = TestContext.Current.CancellationToken;
        var token = _secrets.GetChangeToken();

        await _secrets.RotateSigningKeyAsync(ct);

        token.HasChanged.Should().BeTrue();
        _secrets.GetChangeToken().HasChanged.Should().BeFalse();
    }

    [Fact]
    public async Task A_replaced_key_is_dropped_once_every_token_it_signed_has_expired()
    {
        var ct = TestContext.Current.CancellationToken;
        var id = await _secrets.RotateSigningKeyAsync(ct);

        _time.Advance(Koppel.Core.Auth.KoppelAuthServer.AccessTokenLifetime);
        _secrets.ValidationKeys.Should().HaveCount(2, "a token signed just before the rotation is still valid");

        _time.Advance(ServerSecretsStore.RetiredKeyMargin);

        await EventuallyAsync(() => _secrets.ValidationKeys.Count == 1, ct);
        _secrets.ValidationKeys.Single().KeyId.Should().Be(id);
        _secrets.KeysView.RetiredKeysUntil.Should().BeNull();
        _secrets.Dispose();
        _secrets = await StartStoreAsync();
        _secrets.ValidationKeys.Select(k => k.KeyId).Should().Equal(id);
    }

    [Fact]
    public async Task A_restart_during_the_overlap_keeps_both_keys()
    {
        var ct = TestContext.Current.CancellationToken;
        var id = await _secrets.RotateSigningKeyAsync(ct);

        _secrets.Dispose();
        _secrets = await StartStoreAsync();

        _secrets.SigningKey.KeyId.Should().Be(id);
        _secrets.ValidationKeys.Select(k => k.KeyId).Should().Equal(id, "koppel-1");
    }

    [Fact]
    public async Task PrepareCertificateAsync_keeps_presenting_the_current_certificate_and_survives_a_restart()
    {
        var ct = TestContext.Current.CancellationToken;
        var current = _secrets.Fingerprint;

        var next = await _secrets.PrepareCertificateAsync(ct);

        next.Should().NotBeNull().And.NotBe(current);
        _secrets.Fingerprint.Should().Be(current);
        _secrets.NextFingerprint.Should().Be(next);
        (await _secrets.PrepareCertificateAsync(ct)).Should().BeNull("one is already prepared");

        _secrets.Dispose();
        _secrets = await StartStoreAsync();
        _secrets.Fingerprint.Should().Be(current);
        _secrets.NextFingerprint.Should().Be(next);
    }

    [Fact]
    public async Task CancelCertificateAsync_drops_the_prepared_certificate()
    {
        var ct = TestContext.Current.CancellationToken;
        var current = _secrets.Fingerprint;
        await _secrets.PrepareCertificateAsync(ct);

        (await _secrets.CancelCertificateAsync(ct)).Should().BeTrue();

        _secrets.NextFingerprint.Should().BeNull();
        _secrets.Fingerprint.Should().Be(current);
        (await _secrets.CancelCertificateAsync(ct)).Should().BeFalse();
    }

    [Fact]
    public async Task SwitchCertificateAsync_presents_the_new_certificate_and_names_the_clients_that_lacked_it()
    {
        var ct = TestContext.Current.CancellationToken;
        var (bench1, _) = await _secrets.CreateClientAsync("bench-1", ct);
        await _secrets.CreateClientAsync("bench-2", ct);
        var current = _secrets.Fingerprint;
        var next = (await _secrets.PrepareCertificateAsync(ct))!;
        await _secrets.MarkNextCertificateReceivedAsync(bench1.Id, next.Replace(":", "", StringComparison.Ordinal).ToLowerInvariant(), ct);
        _secrets.Clients.Single(c => c.Id == bench1.Id).HasNextCertificate.Should().BeTrue();

        var done = await _secrets.SwitchCertificateAsync(ct);

        done.Should().BeEquivalentTo(new CertificateSwitch(current, next, ["bench-2"]));
        _secrets.Fingerprint.Should().Be(next);
        ServerCertificate.Fingerprint(_secrets.Certificate).Should().Be(next);
        _secrets.NextFingerprint.Should().BeNull();
        _secrets.Clients.Should().OnlyContain(c => !c.HasNextCertificate);
        (await _secrets.SwitchCertificateAsync(ct)).Should().BeNull("nothing is prepared any more");
    }

    [Fact]
    public async Task MarkNextCertificateReceivedAsync_ignores_a_fingerprint_that_is_not_the_prepared_one()
    {
        var ct = TestContext.Current.CancellationToken;
        var (bench, _) = await _secrets.CreateClientAsync("bench-1", ct);
        await _secrets.PrepareCertificateAsync(ct);

        await _secrets.MarkNextCertificateReceivedAsync(bench.Id, _secrets.Fingerprint, ct);
        await _secrets.MarkNextCertificateReceivedAsync("client-unknown", _secrets.NextFingerprint!, ct);

        _secrets.Clients.Single().HasNextCertificate.Should().BeFalse();
    }

    [Fact]
    public async Task KeysChanged_is_raised_when_the_certificates_change_and_not_for_a_client_change()
    {
        var ct = TestContext.Current.CancellationToken;
        var raised = new List<ServerKeysView>();
        _secrets.KeysChanged += keys =>
        {
            raised.Add(keys);
            return Task.CompletedTask;
        };

        await _secrets.CreateClientAsync("bench-1", ct);
        raised.Should().BeEmpty();

        var next = await _secrets.PrepareCertificateAsync(ct);
        raised.Should().ContainSingle().Which.NextFingerprint.Should().Be(next);
    }

    private async Task<ServerSecretsStore> StartStoreAsync()
    {
        var store = new ServerSecretsStore(new KoppelPaths(Path.Combine(_directory, "config"), Path.Combine(_directory, "keys")), _box, _time, NullLogger<ServerSecretsStore>.Instance);
        await store.StartAsync(TestContext.Current.CancellationToken);
        return store;
    }

    private static async Task EventuallyAsync(Func<bool> condition, CancellationToken ct)
    {
        var until = DateTime.UtcNow.AddSeconds(10);
        while (!condition())
        {
            if (DateTime.UtcNow > until)
            {
                throw new TimeoutException("The condition did not hold in 10 seconds.");
            }

            await Task.Delay(20, ct);
        }
    }
}

/// <summary>The same through a running host: tokens across a rotation, and the endpoints the UI calls.</summary>
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
        var before = await TokenAsync(id, secret, ct);

        using var rotate = await ui.PostAsync("/api/server/keys/signing-key/rotate", null, ct);

        rotate.StatusCode.Should().Be(HttpStatusCode.Accepted);
        var after = await TokenAsync(id, secret, ct);
        KeyIdOf(before).Should().Be("koppel-1");
        KeyIdOf(after).Should().NotBe("koppel-1").And.Be(_koppel.Services.GetRequiredService<ServerSecretsStore>().SigningKey.KeyId);
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
        using var response = await https.PostAsync("/connect/token", new FormUrlEncodedContent(new Dictionary<string, string>
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
        using var request = new HttpRequestMessage(HttpMethod.Get, "/api/remote/hello");
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
