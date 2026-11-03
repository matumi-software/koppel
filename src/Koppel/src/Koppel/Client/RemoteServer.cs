using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Security.Authentication;
using Koppel.Core.Auth;
using Koppel.Core.Security;
using Koppel.Server;

namespace Koppel.Client;

/// <summary>Talks to the configured Koppel server over HTTPS, pinned to its fingerprint, with a bearer token.</summary>
internal sealed partial class RemoteServer(ConnectionStore connection, TimeProvider time, ILogger<RemoteServer> logger)
{
    /// <summary>
    /// Gets a token and calls the server's hello endpoint. The answer is one of a fixed set of outcomes
    /// with fixed wording; what went wrong in detail is logged here and never sent to the caller, so
    /// the test cannot be used to probe addresses and ports.
    /// </summary>
    public async Task<ConnectionTestResult> TestAsync(CancellationToken cancellationToken)
    {
        var settings = connection.Current;
        if (!settings.IsComplete)
        {
            return ConnectionTestResult.Of(ConnectionTestOutcome.Incomplete);
        }

        var server = new Uri(settings.ServerUrl!);
        using var handler = ServerCertificate.CreatePinnedHandler(settings.Trusts);
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
                return ConnectionTestResult.Of(ConnectionTestOutcome.ClientUnknown);
            }

            response.EnsureSuccessStatusCode();
            var hello = await response.Content.ReadFromJsonAsync<RemoteHello>(cancellationToken);
            return ConnectionTestResult.Of(ConnectionTestOutcome.Ok, $"Connected to {hello?.Server} as {hello?.ClientId}.");
        }
        catch (TokenRequestException ex) when (ex.Error == "invalid_client")
        {
            return ConnectionTestResult.Of(ConnectionTestOutcome.RejectedCredentials);
        }
        catch (HttpRequestException ex) when (ex.InnerException is AuthenticationException)
        {
            LogTestFailed(ex, server.Authority);
            return ConnectionTestResult.Of(ConnectionTestOutcome.CertificateMismatch);
        }
        catch (Exception ex) when (ex is HttpRequestException or TaskCanceledException or TokenRequestException or System.Text.Json.JsonException)
        {
            if (cancellationToken.IsCancellationRequested)
            {
                throw;
            }

            LogTestFailed(ex, server.Authority);
            return ConnectionTestResult.Of(ConnectionTestOutcome.Unreachable);
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "The connection test to {Server} failed.")]
    private partial void LogTestFailed(Exception exception, string server);
}

/// <summary>How a connection test ended. A fixed set: the UI shows the message, tests and scripts read the outcome.</summary>
internal enum ConnectionTestOutcome
{
    Ok,
    Incomplete,
    Unreachable,
    CertificateMismatch,
    RejectedCredentials,
    ClientUnknown,
}

internal sealed record ConnectionTestResult(bool Ok, ConnectionTestOutcome Outcome, string Message)
{
    public static ConnectionTestResult Of(ConnectionTestOutcome outcome, string? ok = null) => new(outcome == ConnectionTestOutcome.Ok, outcome, outcome switch
    {
        ConnectionTestOutcome.Ok => ok ?? "Connected.",
        ConnectionTestOutcome.Incomplete => "Fill in the server address, fingerprint, client ID and secret first.",
        ConnectionTestOutcome.Unreachable => "Could not reach the server. Check the address and that the server is running.",
        ConnectionTestOutcome.CertificateMismatch => "The server's certificate does not match the fingerprint. Copy the fingerprint from the server's Clients section again.",
        ConnectionTestOutcome.RejectedCredentials => "The server refused the client ID or secret. Check them, or ask for a new client.",
        _ => "The server accepted the token but no longer knows this client. Ask for a new client.",
    });
}
