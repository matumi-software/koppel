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

        MapKeyEndpoints(group);
        return app;
    }

    /// <summary>
    /// The certificate and the token signing key. Each change answers 202; the new state reaches the UI over
    /// SignalR (<c>ServerKeysChanged</c>) once <c>secrets.json</c> is written, and the paired clients hear of
    /// the next certificate over the remote hub. A change the current state does not allow answers 400.
    /// </summary>
    private static void MapKeyEndpoints(RouteGroupBuilder group)
    {
        var machine = new EntityRef("machine", Environment.MachineName, "machine");

        group.MapGet("/keys", (ServerSecretsStore secrets) => TypedResults.Ok(secrets.KeysView));

        group.MapPost("/keys/signing-key/rotate", async Task<IResult> (ServerSecretsStore secrets, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var id = await secrets.RotateSigningKeyAsync(cancellationToken);
            await activity.RecordAsync(machine, ActivityReason.SigningKeyRotated, ActivityOutcome.Succeeded, $"The token signing key was rotated, to {id}");
            return TypedResults.Accepted((string?)null);
        });

        group.MapPost("/keys/next-certificate", async Task<IResult> (ServerSecretsStore secrets, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            if (await secrets.PrepareCertificateAsync(cancellationToken) is not { } next)
            {
                return Refused("A new certificate is already prepared. Switch to it or cancel it first.");
            }

            await activity.RecordAsync(machine, ActivityReason.CertificatePrepared, ActivityOutcome.Succeeded, $"A new certificate was prepared, fingerprint {Short(next)}");
            return TypedResults.Accepted((string?)null);
        });

        group.MapDelete("/keys/next-certificate", async Task<IResult> (ServerSecretsStore secrets, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var next = secrets.NextFingerprint;
            if (next is null || !await secrets.CancelCertificateAsync(cancellationToken))
            {
                return Refused("There is no new certificate to cancel.");
            }

            await activity.RecordAsync(machine, ActivityReason.CertificateCancelled, ActivityOutcome.Succeeded, $"The new certificate {Short(next)} was cancelled");
            return TypedResults.Accepted((string?)null);
        });

        group.MapPost("/keys/next-certificate/switch", async Task<IResult> (ServerSecretsStore secrets, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            if (await secrets.SwitchCertificateAsync(cancellationToken) is not { } done)
            {
                return Refused("There is no new certificate to switch to. Prepare one first.");
            }

            var summary = done.NotReceived.Count == 0
                ? $"The server switched to its new certificate, fingerprint {Short(done.To)}"
                : $"The server switched to its new certificate, fingerprint {Short(done.To)}. Not received by {string.Join(", ", done.NotReceived)}";
            await activity.RecordAsync(machine, ActivityReason.CertificateSwitched, ActivityOutcome.Succeeded, summary);
            return TypedResults.Accepted((string?)null);
        });

        static IResult Refused(string reason) =>
            TypedResults.ValidationProblem(new Dictionary<string, string[]> { ["certificate"] = [reason] });

        // The first four bytes, as the UI shows them, so a line in the log can be matched to the fingerprint.
        static string Short(string fingerprint) => $"{fingerprint[..11]}…";
    }

    /// <summary>
    /// Endpoints remote Koppel clients call with a bearer token, over HTTPS only. A revoked client is
    /// refused at once, even while an earlier token has not expired, and so is a token issued before the
    /// client's secret was rotated.
    /// </summary>
    public static RouteGroupBuilder MapRemoteEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/remote").RequireAuthorization(KoppelAuthServer.ClientPolicy);
        group.AddEndpointFilter(async (context, next) =>
        {
            var secrets = context.HttpContext.RequestServices.GetRequiredService<ServerSecretsStore>();
            return secrets.IsCurrent(context.HttpContext.User)
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
