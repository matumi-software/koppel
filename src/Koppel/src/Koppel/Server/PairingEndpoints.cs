using Koppel.Core.Activity;
using Skakel.Server;

namespace Koppel.Server;

internal sealed record PairApproveRequest(string? Code);

internal static class PairingEndpoints
{
    /// <summary>
    /// UI endpoints for the server role. A client's side of pairing is Skakel's, at <c>/skakel/v1/pair</c>.
    /// Approving with the right code answers 202; the request leaving <c>PairingRequestsChanged</c> and the
    /// new client in <c>ClientsChanged</c> confirm it. A wrong code is a validation error at once.
    /// </summary>
    public static IEndpointRouteBuilder MapServerPairingEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/server/pairing");
        group.MapGet("/", (SkakelPairing pairing) => TypedResults.Ok(pairing.Overview));

        group.MapPost("/{requestId}/approve", async Task<IResult> (string requestId, PairApproveRequest request, SkakelPairing pairing, CancellationToken cancellationToken) =>
            await pairing.ApproveAsync(requestId, request.Code ?? "", cancellationToken) switch
            {
                SkakelApproval.Approved => TypedResults.Accepted((string?)null),
                SkakelApproval.WrongCode => TypedResults.ValidationProblem(new Dictionary<string, string[]>
                {
                    ["code"] = ["That is not the code the client shows. Check it on the client and type it again."],
                }),
                _ => TypedResults.NotFound(),
            });

        group.MapPost("/{requestId}/deny", async Task<IResult> (string requestId, SkakelPairing pairing) =>
            await pairing.DenyAsync(requestId) ? TypedResults.Accepted((string?)null) : TypedResults.NotFound());

        return app;
    }
}

/// <summary>Puts each pairing that was approved or refused in the activity log.</summary>
internal sealed class PairingActivity(SkakelPairing pairing, IActivityRecorder activity) : IHostedService
{
    public Task StartAsync(CancellationToken cancellationToken)
    {
        pairing.Decided += RecordAsync;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        pairing.Decided -= RecordAsync;
        return Task.CompletedTask;
    }

    private async Task RecordAsync(SkakelPairingDecision decision)
    {
        if (decision.ClientId is { } clientId)
        {
            await activity.RecordAsync(new EntityRef(clientId, decision.ClientName, "client"), ActivityReason.ClientAdded, ActivityOutcome.Succeeded, $"{decision.ClientName} paired from {decision.From}");
        }
        else
        {
            await activity.RecordAsync(new EntityRef(decision.RequestId, decision.ClientName, "client"), ActivityReason.ClientAdded, ActivityOutcome.Failed, $"Pairing with {decision.ClientName} was refused", decision.Refusal);
        }
    }
}
