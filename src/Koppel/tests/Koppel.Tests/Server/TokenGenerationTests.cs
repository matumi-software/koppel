using System.Net;
using System.Net.Http.Json;
using System.Text;
using System.Text.Json;
using Koppel.Core.Auth;
using Microsoft.Extensions.DependencyInjection;

namespace Koppel.Tests.Server;

/// <summary>A token is issued for the generation that validation verified, never one read later.</summary>
public sealed class TokenGenerationTests : IDisposable
{
    private readonly KoppelFactory _koppel;
    private readonly RotatingValidator _validator = new();

    public TokenGenerationTests() => _koppel = new KoppelFactory(services: s => s.AddSingleton<IClientCredentialValidator>(_validator));

    public void Dispose() => _koppel.Dispose();

    [Fact]
    public async Task A_token_carries_the_generation_its_secret_was_verified_at_even_if_the_client_is_rotated_meanwhile()
    {
        var ct = TestContext.Current.CancellationToken;
        using var https = _koppel.CreateHttpsClient();

        using var response = await https.PostAsync("/connect/token", new FormUrlEncodedContent(new Dictionary<string, string>
        {
            ["grant_type"] = "client_credentials",
            ["client_id"] = "bench",
            ["client_secret"] = "old-secret",
        }), ct);

        response.StatusCode.Should().Be(HttpStatusCode.OK);
        var token = (await response.Content.ReadFromJsonAsync<JsonElement>(ct)).GetProperty("access_token").GetString()!;
        var payload = token.Split('.')[1].Replace('-', '+').Replace('_', '/');
        payload = payload.PadRight(payload.Length + (4 - payload.Length % 4) % 4, '=');
        var claims = JsonDocument.Parse(Encoding.UTF8.GetString(Convert.FromBase64String(payload))).RootElement;
        claims.GetProperty(KoppelAuthServer.GenerationClaim).GetString().Should().Be("1", "the secret was verified at generation 1; the rotation to 2 came after");
        _validator.Generation.Should().Be(2);
    }

    /// <summary>Verifies a secret at generation 1, then rotates the client before it returns, as a concurrent rotate would.</summary>
    private sealed class RotatingValidator : IClientCredentialValidator
    {
        public int Generation { get; private set; } = 1;

        public ValueTask<int?> ValidateAsync(string clientId, string clientSecret, CancellationToken cancellationToken)
        {
            if (clientSecret != "old-secret")
            {
                return ValueTask.FromResult<int?>(null);
            }

            var verified = Generation;
            Generation++;
            return ValueTask.FromResult<int?>(verified);
        }
    }
}
