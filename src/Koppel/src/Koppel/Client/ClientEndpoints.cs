using System.Globalization;

namespace Koppel.Client;

/// <summary>A change to the connection. A null or empty secret keeps the stored one.</summary>
internal sealed record ConnectionUpdate(string? ServerUrl, string? ServerFingerprint, string? ClientId, string? ClientSecret)
{
    public Dictionary<string, string[]> Validate()
    {
        var errors = new Dictionary<string, string[]>();
        if (!Uri.TryCreate(ServerUrl, UriKind.Absolute, out var url) || url.Scheme != Uri.UriSchemeHttps)
        {
            errors["serverUrl"] = ["Use the server's HTTPS address, such as https://lab-rack-3:5241."];
        }

        var hex = new string((ServerFingerprint ?? "").Where(Uri.IsHexDigit).ToArray());
        if (hex.Length != 64)
        {
            errors["serverFingerprint"] = ["Paste the SHA-256 fingerprint shown in the server's Clients section."];
        }

        if (string.IsNullOrWhiteSpace(ClientId))
        {
            errors["clientId"] = ["Enter the client ID the server gave you."];
        }

        return errors;
    }

    public ClientConnection ApplyTo(ClientConnection current) => current with
    {
        ServerUrl = new Uri(ServerUrl!).GetLeftPart(UriPartial.Authority),
        ServerFingerprint = Normalize(ServerFingerprint!),
        ClientId = ClientId!.Trim(),
        ClientSecret = string.IsNullOrEmpty(ClientSecret) ? current.ClientSecret : ClientSecret,
    };

    private static string Normalize(string fingerprint)
    {
        var hex = new string(fingerprint.Where(Uri.IsHexDigit).ToArray()).ToUpperInvariant();
        return string.Join(':', Enumerable.Range(0, hex.Length / 2).Select(i => hex.Substring(i * 2, 2).ToString(CultureInfo.InvariantCulture)));
    }
}

internal static class ClientEndpoints
{
    /// <summary>
    /// UI endpoints for the client role. PUT answers 202 with no body; the stored connection reaches the
    /// UI over SignalR (<c>ConnectionChanged</c>) once <c>connection.json</c> is written. The secret is
    /// never sent back. Testing the connection is a query, so it answers with its result.
    /// </summary>
    public static IEndpointRouteBuilder MapClientEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/client");

        group.MapGet("/connection", (ConnectionStore store) => TypedResults.Ok(store.View));

        group.MapPut("/connection", async Task<IResult> (ConnectionUpdate update, ConnectionStore store, CancellationToken cancellationToken) =>
        {
            var errors = update.Validate();
            if (errors.Count > 0)
            {
                return TypedResults.ValidationProblem(errors);
            }

            await store.UpdateAsync(update.ApplyTo, cancellationToken);
            return TypedResults.Accepted((string?)null);
        });

        group.MapPost("/connection/test", async (RemoteServer server, CancellationToken cancellationToken) =>
            TypedResults.Ok(await server.TestAsync(cancellationToken)));

        return app;
    }
}
