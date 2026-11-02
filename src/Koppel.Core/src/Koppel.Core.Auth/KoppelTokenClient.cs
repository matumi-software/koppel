using System.Net.Http.Json;
using System.Text.Json.Serialization;

namespace Koppel.Core.Auth;

/// <summary>A token request the server refused, with OAuth's error code and description.</summary>
public sealed class TokenRequestException(string error, string? description)
    : Exception(description is null ? error : $"{error}: {description}")
{
    public string Error { get; } = error;
}

/// <summary>
/// Gets client-credentials access tokens from a Koppel server and reuses each until shortly before
/// it expires. One instance per server connection.
/// </summary>
public sealed class KoppelTokenClient(HttpClient http, TimeProvider time) : IDisposable
{
    private static readonly TimeSpan RenewBefore = TimeSpan.FromMinutes(1);
    private readonly SemaphoreSlim _gate = new(1, 1);
    private (string ClientId, string Token, DateTimeOffset ExpiresAt)? _cached;

    /// <param name="server">The server's HTTPS base address, such as <c>https://lab-rack-3:5241</c>.</param>
    public async Task<string> GetAccessTokenAsync(Uri server, string clientId, string clientSecret, CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken);
        try
        {
            if (_cached is { } cached && cached.ClientId == clientId && cached.ExpiresAt - RenewBefore > time.GetUtcNow())
            {
                return cached.Token;
            }

            using var form = new FormUrlEncodedContent(new Dictionary<string, string>
            {
                ["grant_type"] = "client_credentials",
                ["client_id"] = clientId,
                ["client_secret"] = clientSecret,
            });
            using var response = await http.PostAsync(new Uri(server, KoppelAuthServer.TokenPath), form, cancellationToken);
            var body = await response.Content.ReadFromJsonAsync<TokenResponse>(cancellationToken)
                ?? throw new TokenRequestException("invalid_response", "The server sent an empty token response.");

            if (!response.IsSuccessStatusCode || body.AccessToken is null)
            {
                throw new TokenRequestException(body.Error ?? "invalid_response", body.ErrorDescription);
            }

            _cached = (clientId, body.AccessToken, time.GetUtcNow().AddSeconds(body.ExpiresIn ?? 300));
            return body.AccessToken;
        }
        finally
        {
            _gate.Release();
        }
    }

    /// <summary>Drops the cached token, for example after the server answered 401.</summary>
    public void Forget() => _cached = null;

    public void Dispose() => _gate.Dispose();

    private sealed record TokenResponse(
        [property: JsonPropertyName("access_token")] string? AccessToken,
        [property: JsonPropertyName("expires_in")] int? ExpiresIn,
        [property: JsonPropertyName("error")] string? Error,
        [property: JsonPropertyName("error_description")] string? ErrorDescription);
}
