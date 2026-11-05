using Koppel.Core.Activity;
using Koppel.Usb;
using Microsoft.AspNetCore.Http.HttpResults;
using Skakel.Server;

namespace Koppel.Server;

internal sealed record CreateClientRequest(string? Name);

/// <summary>Returned when a client is created. The secret is also kept, encrypted, so it can be copied again later.</summary>
internal sealed record CreatedClient(string ClientId, string ClientSecret, string Name);

internal sealed record ServerIdentity(string Name, string Fingerprint, int HttpsPort);

internal sealed record ClientSecretResponse(string ClientSecret);

internal static class ServerEndpoints
{
    /// <summary>
    /// UI endpoints for the server role: the server's identity and its registered clients, which Skakel
    /// keeps. Creating a client answers 201 with the one-time secret; the client list itself reaches the
    /// UI over SignalR (<c>ClientsChanged</c>) once <c>secrets.json</c> is written. Revoking answers 202.
    /// </summary>
    public static IEndpointRouteBuilder MapServerEndpoints(this IEndpointRouteBuilder app, int httpsPort)
    {
        var group = app.MapGroup("/api/server");

        group.MapGet("/identity", (SkakelKeys keys) =>
            TypedResults.Ok(new ServerIdentity(Environment.MachineName, keys.Fingerprint, httpsPort)));

        group.MapGet("/clients", (SkakelClients clients) => TypedResults.Ok(clients.All));

        group.MapPost("/clients", async Task<IResult> (CreateClientRequest request, SkakelClients clients, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var name = request.Name?.Trim();
            if (string.IsNullOrEmpty(name) || name.Length > 80)
            {
                return NameProblem("Give the client a name of up to 80 characters, such as the machine it runs on.");
            }

            SkakelNewClient client;
            try
            {
                client = await clients.RegisterAsync(name, cancellationToken);
            }
            catch (ArgumentException)
            {
                // Skakel takes control and format characters out of the name, and refuses one with nothing left.
                return NameProblem("Nothing is left of the name once invisible characters are taken out. Type a name such as the machine it runs on.");
            }

            await activity.RecordAsync(new EntityRef(client.ClientId, client.Name, "client"), ActivityReason.ClientAdded, ActivityOutcome.Succeeded, $"Client {client.Name} was added");
            return TypedResults.Created($"/api/server/clients/{client.ClientId}", new CreatedClient(client.ClientId, client.ClientSecret, client.Name));
        });

        group.MapDelete("/clients/{clientId}", async Task<IResult> (string clientId, SkakelClients clients, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var name = clients.NameOf(clientId) ?? clientId;
            if (!await clients.RevokeAsync(clientId, cancellationToken))
            {
                return TypedResults.NotFound();
            }

            await activity.RecordAsync(new EntityRef(clientId, name, "client"), ActivityReason.ClientRevoked, ActivityOutcome.Succeeded, $"Client {name} was revoked");
            return TypedResults.Accepted((string?)null);
        });

        // One client's secret, fetched only when someone reveals or copies it.
        group.MapGet("/clients/{clientId}/secret", IResult (string clientId, SkakelClients clients) =>
        {
            if (clients.NameOf(clientId) is null)
            {
                return TypedResults.NotFound();
            }

            return clients.SecretOf(clientId) is { } secret
                ? TypedResults.Ok(new ClientSecretResponse(secret))
                : TypedResults.Conflict(new { error = "This client was added before secrets were kept. Rotate the secret to get one you can copy." });
        });

        // A new secret, returned once in the response; the client list follows over SignalR.
        group.MapPost("/clients/{clientId}/rotate", async Task<IResult> (string clientId, SkakelClients clients, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var secret = await clients.RotateSecretAsync(clientId, cancellationToken);
            if (secret is null)
            {
                return TypedResults.NotFound();
            }

            var name = clients.NameOf(clientId) ?? clientId;
            await activity.RecordAsync(new EntityRef(clientId, name, "client"), ActivityReason.SecretRotated, ActivityOutcome.Succeeded, $"The secret for {name} was rotated");
            return TypedResults.Ok(new ClientSecretResponse(secret));
        });

        MapKeyEndpoints(group);
        return app;
    }

    private static ValidationProblem NameProblem(string message) =>
        TypedResults.ValidationProblem(new Dictionary<string, string[]> { ["name"] = [message] });

    /// <summary>
    /// The certificate and the token signing key. Each change answers 202; the new state reaches the UI over
    /// SignalR (<c>ServerKeysChanged</c>) once <c>secrets.json</c> is written, and Skakel tells the paired
    /// clients of the next certificate over the remote hub. A change the current state does not allow answers 400.
    /// </summary>
    private static void MapKeyEndpoints(RouteGroupBuilder group)
    {
        var machine = new EntityRef("machine", Environment.MachineName, "machine");

        group.MapGet("/keys", (SkakelKeys keys) => TypedResults.Ok(keys.View));

        group.MapPost("/keys/signing-key/rotate", async Task<IResult> (SkakelKeys keys, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var id = await keys.RotateSigningKeyAsync(cancellationToken);
            await activity.RecordAsync(machine, ActivityReason.SigningKeyRotated, ActivityOutcome.Succeeded, $"The token signing key was rotated, to {id}");
            return TypedResults.Accepted((string?)null);
        });

        group.MapPost("/keys/next-certificate", async Task<IResult> (SkakelKeys keys, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            if (await keys.PrepareNextCertificateAsync(cancellationToken) is not { } next)
            {
                return Refused("A new certificate is already prepared. Switch to it or cancel it first.");
            }

            await activity.RecordAsync(machine, ActivityReason.CertificatePrepared, ActivityOutcome.Succeeded, $"A new certificate was prepared, fingerprint {Short(next)}");
            return TypedResults.Accepted((string?)null);
        });

        group.MapDelete("/keys/next-certificate", async Task<IResult> (SkakelKeys keys, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            var next = keys.NextFingerprint;
            if (next is null || !await keys.CancelNextCertificateAsync(cancellationToken))
            {
                return Refused("There is no new certificate to cancel.");
            }

            await activity.RecordAsync(machine, ActivityReason.CertificateCancelled, ActivityOutcome.Succeeded, $"The new certificate {Short(next)} was cancelled");
            return TypedResults.Accepted((string?)null);
        });

        group.MapPost("/keys/next-certificate/switch", async Task<IResult> (SkakelKeys keys, IActivityRecorder activity, CancellationToken cancellationToken) =>
        {
            if (await keys.SwitchCertificateAsync(cancellationToken) is not { } done)
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
    /// Koppel's own endpoints for remote clients, beside Skakel's: a bearer token from a client still
    /// registered at its current generation, over HTTPS. Skakel's <c>/skakel/v1/hello</c> is what a
    /// client calls to check its credentials.
    /// </summary>
    public static RouteGroupBuilder MapRemoteEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/remote").RequireSkakelClient();
        group.MapGet("/devices", (UsbServer usb) => TypedResults.Ok(usb.Current.ForRemote()));
        return group;
    }
}
