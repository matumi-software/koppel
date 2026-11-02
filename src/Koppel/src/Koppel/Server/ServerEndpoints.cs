using Koppel.Core.Activity;
using Koppel.Core.Auth;

namespace Koppel.Server;

internal sealed record CreateClientRequest(string? Name);

/// <summary>Returned when a client is created. The secret is also kept, encrypted, so it can be copied again later.</summary>
internal sealed record CreatedClient(string ClientId, string ClientSecret, string Name);

internal sealed record ServerIdentity(string Name, string Fingerprint, int HttpsPort);

internal sealed record ClientSecretResponse(string ClientSecret);

internal static class ServerEndpoints
{
    /// <summary>
    /// UI endpoints for the server role: the server's identity and its registered clients. Creating
    /// a client answers 201 with the one-time secret; the client list itself reaches the UI over
    /// SignalR (<c>ClientsChanged</c>) once <c>secrets.json</c> is written. Revoking answers 202.
    /// </summary>
    public static IEndpointRouteBuilder MapServerEndpoints(this IEndpointRouteBuilder app, int httpsPort)
    {
        var group = app.MapGroup("/api/server");

        group.MapGet("/identity", (ServerSecretsStore secrets) =>
            TypedResults.Ok(new ServerIdentity(Environment.MachineName, secrets.Fingerprint, httpsPort)));

        group.MapGet("/clients", (ServerSecretsStore secrets) => TypedResults.Ok(secrets.Clients));

        group.MapPost("/clients", async Task<IResult> (CreateClientRequest request, ServerSecretsStore secrets, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var name = request.Name?.Trim();
            if (string.IsNullOrEmpty(name) || name.Length > 80)
            {
                return TypedResults.ValidationProblem(new Dictionary<string, string[]>
                {
                    ["name"] = ["Give the client a name of up to 80 characters, such as the machine it runs on."],
                });
            }

            var (client, secret) = await secrets.CreateClientAsync(name, cancellationToken);
            await activity.RecordAsync(new EntityRef(client.Id, client.Name, "client"), ActivityReason.ClientAdded, ActivityOutcome.Succeeded, $"Client {client.Name} was added");
            return TypedResults.Created($"/api/server/clients/{client.Id}", new CreatedClient(client.Id, secret, client.Name));
        });

        group.MapDelete("/clients/{clientId}", async Task<IResult> (string clientId, ServerSecretsStore secrets, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var name = secrets.NameOf(clientId) ?? clientId;
            if (!await secrets.RevokeClientAsync(clientId, cancellationToken))
            {
                return TypedResults.NotFound();
            }

            await activity.RecordAsync(new EntityRef(clientId, name, "client"), ActivityReason.ClientRevoked, ActivityOutcome.Succeeded, $"Client {name} was revoked");
            return TypedResults.Accepted((string?)null);
        });

        // One client's secret, fetched only when someone reveals or copies it.
        group.MapGet("/clients/{clientId}/secret", IResult (string clientId, ServerSecretsStore secrets) =>
        {
            try
            {
                return secrets.SecretOf(clientId) is { } secret
                    ? TypedResults.Ok(new ClientSecretResponse(secret))
                    : TypedResults.Conflict(new { error = "This client was added before secrets were kept. Rotate the secret to get one you can copy." });
            }
            catch (KeyNotFoundException)
            {
                return TypedResults.NotFound();
            }
        });

        // A new secret, returned once in the response; the client list follows over SignalR.
        group.MapPost("/clients/{clientId}/rotate", async Task<IResult> (string clientId, ServerSecretsStore secrets, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var secret = await secrets.RotateSecretAsync(clientId, cancellationToken);
            if (secret is null)
            {
                return TypedResults.NotFound();
            }

            var name = secrets.NameOf(clientId) ?? clientId;
            await activity.RecordAsync(new EntityRef(clientId, name, "client"), ActivityReason.SecretRotated, ActivityOutcome.Succeeded, $"The secret for {name} was rotated");
            return TypedResults.Ok(new ClientSecretResponse(secret));
        });

        return app;
    }

    /// <summary>
    /// Endpoints remote Koppel clients call with a bearer token, over HTTPS only. A revoked client is
    /// refused at once, even while an earlier token has not expired.
    /// </summary>
    public static RouteGroupBuilder MapRemoteEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/remote").RequireAuthorization(KoppelAuthServer.ClientPolicy);
        group.AddEndpointFilter(async (context, next) =>
        {
            var secrets = context.HttpContext.RequestServices.GetRequiredService<ServerSecretsStore>();
            return secrets.IsRegistered(context.HttpContext.User.GetClientId())
                ? await next(context)
                : TypedResults.Unauthorized();
        });

        group.MapGet("/hello", (HttpContext http) =>
            TypedResults.Ok(new RemoteHello(Environment.MachineName, http.User.GetClientId()!)));

        return group;
    }
}

/// <summary>What a client sees when its credentials work.</summary>
internal sealed record RemoteHello(string Server, string ClientId);
