namespace Koppel.Server;

internal sealed record PairOpenRequest(string? ClientName, string? Commitment);

/// <param name="PollToken">Only the opener gets it. Poll and withdraw need it, in the <c>X-Pair-Token</c> header.</param>
internal sealed record PairOpened(string RequestId, string ServerNonce, DateTimeOffset ExpiresAt, string PollToken);

internal sealed record PairRevealRequest(string? ClientNonce);

internal sealed record PairApproveRequest(string? Code);

internal static class PairingEndpoints
{
    public const string TokenHeader = "X-Pair-Token";

    /// <summary>
    /// Endpoints a client calls to pair, before it has credentials: no token, but on the HTTPS port only
    /// and not in the client role (see <see cref="Web.SurfaceGuard"/>), so the client has pinned the
    /// certificate it will derive the code from. See <see cref="PairingStore"/>.
    /// </summary>
    public static IEndpointRouteBuilder MapPairingEndpoints(this IEndpointRouteBuilder app)
    {
        var pair = app.MapGroup("/api/pair");

        pair.MapPost("/", IResult (PairOpenRequest request, PairingStore pairing, HttpContext http) =>
        {
            var name = request.ClientName?.Trim();
            if (string.IsNullOrEmpty(name) || name.Length > 80 || request.Commitment is not { Length: 64 } commitment || !commitment.All(Uri.IsHexDigit))
            {
                return TypedResults.BadRequest();
            }

            return pairing.Open(name, commitment, http.Connection.RemoteIpAddress) is { } opened
                ? TypedResults.Ok(new PairOpened(opened.Id, opened.ServerNonce, opened.ExpiresAt, opened.PollToken))
                : TypedResults.StatusCode(StatusCodes.Status429TooManyRequests);
        });

        pair.MapPost("/{requestId}/reveal", async Task<IResult> (string requestId, PairRevealRequest request, PairingStore pairing) =>
            request.ClientNonce is { Length: > 0 } nonce && await pairing.RevealAsync(requestId, nonce)
                ? TypedResults.NoContent()
                : TypedResults.NotFound());

        pair.MapGet("/{requestId}", async Task<IResult> (string requestId, PairingStore pairing, HttpContext http) =>
            await pairing.PollAsync(requestId, http.Request.Headers[TokenHeader]) is { } poll ? TypedResults.Ok(poll) : TypedResults.NotFound());

        pair.MapDelete("/{requestId}", async Task<IResult> (string requestId, PairingStore pairing, HttpContext http) =>
            await pairing.WithdrawAsync(requestId, http.Request.Headers[TokenHeader]) ? TypedResults.NoContent() : TypedResults.NotFound());

        return app;
    }

    /// <summary>
    /// UI endpoints for the server role. Approving with the right code answers 202; the request leaving
    /// <c>PairingRequestsChanged</c> and the new client in <c>ClientsChanged</c> confirm it. A wrong
    /// code is a validation error at once.
    /// </summary>
    public static IEndpointRouteBuilder MapServerPairingEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/server/pairing");
        group.MapGet("/", (PairingStore pairing) => TypedResults.Ok(pairing.Overview));

        group.MapPost("/{requestId}/approve", async Task<IResult> (string requestId, PairApproveRequest request, PairingStore pairing, CancellationToken cancellationToken) =>
            await pairing.ApproveAsync(requestId, request.Code ?? "", cancellationToken) switch
            {
                null => TypedResults.NotFound(),
                false => TypedResults.ValidationProblem(new Dictionary<string, string[]>
                {
                    ["code"] = ["That is not the code the client shows. Check it on the client and type it again."],
                }),
                true => TypedResults.Accepted((string?)null),
            });

        group.MapPost("/{requestId}/deny", async Task<IResult> (string requestId, PairingStore pairing) =>
            await pairing.DenyAsync(requestId) ? TypedResults.Accepted((string?)null) : TypedResults.NotFound());

        return app;
    }
}
