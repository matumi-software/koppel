using System.Globalization;

namespace Koppel.Client;

/// <summary>A change to the connection. A null or empty secret keeps the stored one, but only while the server stays the same.</summary>
internal sealed record ConnectionUpdate(string? ServerUrl, string? ServerFingerprint, string? ClientId, string? ClientSecret)
{
    public Dictionary<string, string[]> Validate()
    {
        var errors = new Dictionary<string, string[]>();
        if (!Uri.TryCreate(ServerUrl, UriKind.Absolute, out var url) || url.Scheme != Uri.UriSchemeHttps)
        {
            errors["serverUrl"] = ["Use the server's HTTPS address, such as https://lab-rack-3:5240."];
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

    /// <summary>True when the update points at the server the connection already has: same address and same fingerprint.</summary>
    public bool KeepsServer(ClientConnection current) =>
        string.Equals(current.ServerUrl, new Uri(ServerUrl!).GetLeftPart(UriPartial.Authority), StringComparison.OrdinalIgnoreCase)
        && SameFingerprint(current.ServerFingerprint, ServerFingerprint);

    /// <summary>
    /// The stored secret was issued by the stored server, so it carries over only when the server is
    /// unchanged. For another server it never does: the secret would go to a server that did not issue it.
    /// </summary>
    public ClientConnection ApplyTo(ClientConnection current) => current with
    {
        ServerUrl = new Uri(ServerUrl!).GetLeftPart(UriPartial.Authority),
        ServerFingerprint = Normalize(ServerFingerprint!),
        ClientId = ClientId!.Trim(),
        ClientSecret = !string.IsNullOrEmpty(ClientSecret) ? ClientSecret : KeepsServer(current) ? current.ClientSecret : null,
    };

    /// <summary>Fingerprints compare by their hex digits alone.</summary>
    public static bool SameFingerprint(string? a, string? b) =>
        string.Equals(Digits(a), Digits(b), StringComparison.OrdinalIgnoreCase);

    private static string Digits(string? fingerprint) => new((fingerprint ?? "").Where(Uri.IsHexDigit).ToArray());

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

            if (string.IsNullOrEmpty(update.ClientSecret) && !update.KeepsServer(store.Current))
            {
                return TypedResults.ValidationProblem(new Dictionary<string, string[]>
                {
                    ["clientSecret"] = ["Enter the client secret. The stored one only carries over while the server address and fingerprint stay the same."],
                });
            }

            await store.UpdateAsync(update.ApplyTo, cancellationToken);
            return TypedResults.Accepted((string?)null);
        });

        group.MapPost("/connection/test", async (RemoteServer server, CancellationToken cancellationToken) =>
            TypedResults.Ok(await server.TestAsync(cancellationToken)));

        // Servers found on the local network. The list also follows over SignalR (ServersDiscovered).
        group.MapGet("/discovery", (Discovery.ServerFinder finder) => TypedResults.Ok(finder.Servers));

        // Pairing answers 202 and runs in the background; PairingStatusChanged carries the code and the
        // outcome, and a successful pairing ends with ConnectionChanged like any other save.
        group.MapGet("/pairing", (PairingClient pairing) => TypedResults.Ok(pairing.Current));

        group.MapPost("/pairing", IResult (PairStartRequest request, PairingClient pairing) =>
        {
            var errors = new ConnectionUpdate(request.ServerUrl, request.ServerFingerprint, "pairing", null).Validate();
            if (errors.Count > 0)
            {
                return TypedResults.ValidationProblem(errors);
            }

            pairing.Begin(new Uri(request.ServerUrl!).GetLeftPart(UriPartial.Authority), request.ServerFingerprint!, request.ServerName);
            return TypedResults.Accepted((string?)null);
        });

        // The server approved, but saving replaces the connection to another server: the person confirms.
        group.MapPost("/pairing/confirm", async Task<IResult> (PairingClient pairing, CancellationToken cancellationToken) =>
            await pairing.ConfirmAsync(cancellationToken)
                ? TypedResults.Accepted((string?)null)
                : TypedResults.ValidationProblem(new Dictionary<string, string[]> { ["pairing"] = ["There is nothing to confirm. Start pairing again."] }));

        group.MapDelete("/pairing", async (PairingClient pairing) =>
        {
            await pairing.CancelAsync();
            return TypedResults.Accepted((string?)null);
        });

        return app;
    }
}
