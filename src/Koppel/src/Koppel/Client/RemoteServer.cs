using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Security.Authentication;
using Koppel.Core.Auth;
using Koppel.Core.Security;
using Koppel.Server;

namespace Koppel.Client;

/// <summary>Talks to the configured Koppel server over HTTPS, pinned to its fingerprint, with a bearer token.</summary>
internal sealed class RemoteServer(ConnectionStore connection, TimeProvider time)
{
    /// <summary>Gets a token and calls the server's hello endpoint. Returns a sentence for the UI.</summary>
    public async Task<ConnectionTestResult> TestAsync(CancellationToken cancellationToken)
    {
        var settings = connection.Current;
        if (!settings.IsComplete)
        {
            return new(false, "Fill in the server address, fingerprint, client ID and secret first.");
        }

        var server = new Uri(settings.ServerUrl!);
        using var handler = ServerCertificate.CreatePinnedHandler(settings.ServerFingerprint!);
        using var http = new HttpClient(handler) { Timeout = TimeSpan.FromSeconds(10) };
        using var tokens = new KoppelTokenClient(http, time);
        try
        {
            var token = await tokens.GetAccessTokenAsync(server, settings.ClientId!, settings.ClientSecret!, cancellationToken);
            using var request = new HttpRequestMessage(HttpMethod.Get, new Uri(server, "/api/remote/hello"));
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
            using var response = await http.SendAsync(request, cancellationToken);
            if (response.StatusCode == HttpStatusCode.Unauthorized)
            {
                return new(false, "The server accepted the token but no longer knows this client. Ask for a new client.");
            }

            response.EnsureSuccessStatusCode();
            var hello = await response.Content.ReadFromJsonAsync<RemoteHello>(cancellationToken);
            return new(true, $"Connected to {hello?.Server} as {hello?.ClientId}.");
        }
        catch (TokenRequestException ex) when (ex.Error == "invalid_client")
        {
            return new(false, "The server refused the client ID or secret. Check them, or ask for a new client.");
        }
        catch (HttpRequestException ex) when (ex.InnerException is AuthenticationException)
        {
            return new(false, "The server's certificate does not match the fingerprint. Copy the fingerprint from the server's Clients section again.");
        }
        catch (Exception ex) when (ex is HttpRequestException or TaskCanceledException or TokenRequestException)
        {
            return new(false, $"Could not reach {server.Authority}: {ex.Message}");
        }
    }
}

internal sealed record ConnectionTestResult(bool Ok, string Message);
