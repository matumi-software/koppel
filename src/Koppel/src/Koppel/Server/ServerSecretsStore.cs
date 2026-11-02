using System.Security.Claims;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using Koppel.Core.Auth;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Microsoft.IdentityModel.Tokens;

namespace Koppel.Server;

/// <summary>
/// A client allowed to use this server. The secret is kept, inside the encrypted file, so an admin can
/// copy it again later, as Timebase Pulse does. Clients made before that have only the hash.
/// </summary>
internal sealed record RegisteredClient(string Id, string Name, string SecretHash, DateTimeOffset CreatedAt, string? Secret = null, int Generation = 1);

/// <summary>The contents of the encrypted <c>secrets.json</c> (server role).</summary>
internal sealed record ServerSecrets(string? SigningKey, string? CertificatePfx, IReadOnlyList<RegisteredClient> Clients)
{
    public static ServerSecrets Empty { get; } = new(null, null, []);
}

/// <summary>What the UI lists about a client. The secret is fetched one client at a time, on demand.</summary>
/// <param name="HasSecret">False for clients made before secrets were kept: rotate to get a copyable one.</param>
/// <param name="LastSeen">When the client last got a token, since this server started.</param>
public sealed record ClientView(string Id, string Name, DateTimeOffset CreatedAt, bool HasSecret, DateTimeOffset? LastSeen);

/// <summary>
/// Owns <c>secrets.json</c>: the token signing key, the HTTPS certificate and the registered clients.
/// Creates the key and certificate on first start. Same write-then-raise rule as settings.
/// </summary>
internal sealed class ServerSecretsStore : IHostedService, IClientCredentialValidator, ISigningKeySource, IDisposable
{
    private readonly JsonConfigFile<ServerSecrets> _file;
    private readonly TimeProvider _time;
    private readonly System.Collections.Concurrent.ConcurrentDictionary<string, DateTimeOffset> _lastSeen = new();
    private RsaSecurityKey? _signingKey;
    private X509Certificate2? _certificate;

    public ServerSecretsStore(KoppelPaths paths, SecretBox box, TimeProvider time, ILogger<ServerSecretsStore> logger)
    {
        _time = time;
        _file = new JsonConfigFile<ServerSecrets>(paths.ServerSecretsFile, ServerSecrets.Empty, KoppelJson.Files, logger, box.AsFileCodec());
        _file.Changed += secrets => ClientsChanged.RaiseAsync(ToViews(secrets), logger);
    }

    /// <summary>Raised after the client list on disk changed.</summary>
    public event Func<IReadOnlyList<ClientView>, Task>? ClientsChanged;

    public IReadOnlyList<ClientView> Clients => ToViews(_file.Current);

    public SecurityKey SigningKey => _signingKey ?? throw new InvalidOperationException("Server secrets are not loaded yet.");

    public X509Certificate2 Certificate => _certificate ?? throw new InvalidOperationException("Server secrets are not loaded yet.");

    public string Fingerprint => ServerCertificate.Fingerprint(Certificate);

    public async Task StartAsync(CancellationToken cancellationToken)
    {
        await _file.StartAsync(cancellationToken);
        var secrets = _file.Current;
        if (secrets.SigningKey is null || secrets.CertificatePfx is null)
        {
            secrets = await _file.UpdateAsync(current => current with
            {
                SigningKey = current.SigningKey ?? NewSigningKey(),
                CertificatePfx = current.CertificatePfx ?? Convert.ToBase64String(ServerCertificate.CreatePfx(Environment.MachineName)),
            }, cancellationToken);
        }

        var rsa = RSA.Create();
        rsa.ImportPkcs8PrivateKey(Convert.FromBase64String(secrets.SigningKey!), out _);
        _signingKey = new RsaSecurityKey(rsa) { KeyId = "koppel-1" };
        _certificate = ServerCertificate.Load(Convert.FromBase64String(secrets.CertificatePfx!));
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        _file.Stop();
        return Task.CompletedTask;
    }

    /// <summary>Registers a client. The secret is returned and also kept in the encrypted file, so an admin can copy it again; the hash is what tokens are checked against.</summary>
    public async Task<(ClientView Client, string Secret)> CreateClientAsync(string name, CancellationToken cancellationToken)
    {
        var secret = Secrets.NewSecret();
        var client = new RegisteredClient(Secrets.NewId("client"), name.Trim(), Secrets.Hash(secret), _time.GetUtcNow(), secret);
        await _file.UpdateAsync(current => current with { Clients = [.. current.Clients, client] }, cancellationToken);
        return (ToView(client), secret);
    }

    /// <summary>The stored secret, or null when this client predates kept secrets. Throws when unknown.</summary>
    public string? SecretOf(string clientId) =>
        (_file.Current.Clients.FirstOrDefault(c => c.Id == clientId) ?? throw new KeyNotFoundException(clientId)).Secret;

    public string? NameOf(string? clientId) => _file.Current.Clients.FirstOrDefault(c => c.Id == clientId)?.Name;

    /// <summary>
    /// Replaces the secret. The old one stops working for new tokens at once, and every token issued before
    /// stops being accepted, because the client gets a new generation.
    /// </summary>
    public async Task<string?> RotateSecretAsync(string clientId, CancellationToken cancellationToken)
    {
        if (_file.Current.Clients.All(c => c.Id != clientId))
        {
            return null;
        }

        var secret = Secrets.NewSecret();
        await _file.UpdateAsync(current => current with
        {
            Clients = [.. current.Clients.Select(c => c.Id == clientId ? c with { SecretHash = Secrets.Hash(secret), Secret = secret, Generation = c.Generation + 1 } : c)],
        }, cancellationToken);
        return secret;
    }

    /// <summary>Removes a client. Its tokens stop working when they expire; new ones are refused at once.</summary>
    public async Task<bool> RevokeClientAsync(string clientId, CancellationToken cancellationToken)
    {
        if (_file.Current.Clients.All(c => c.Id != clientId))
        {
            return false;
        }

        await _file.UpdateAsync(current => current with { Clients = [.. current.Clients.Where(c => c.Id != clientId)] }, cancellationToken);
        return true;
    }

    public bool IsRegistered(string? clientId) => clientId is not null && _file.Current.Clients.Any(c => c.Id == clientId);

    /// <summary>The client is registered and its credentials have not been rotated since the token was issued.</summary>
    public bool IsCurrent(string? clientId, int? generation) =>
        clientId is not null && generation is not null && _file.Current.Clients.Any(c => c.Id == clientId && c.Generation == generation);

    /// <summary>Whether a validated bearer token belongs to a registered client at its current generation.</summary>
    public bool IsCurrent(ClaimsPrincipal? principal) => principal is not null && IsCurrent(principal.GetClientId(), principal.GetGeneration());

    public async ValueTask<int?> ValidateAsync(string clientId, string clientSecret, CancellationToken cancellationToken)
    {
        var client = _file.Current.Clients.FirstOrDefault(c => c.Id == clientId);
        var ok = client is not null && Secrets.Verify(clientSecret, client.SecretHash);
        if (ok)
        {
            _lastSeen[clientId] = _time.GetUtcNow();
            if (Secrets.IsLegacy(client!.SecretHash))
            {
                await UpgradeHashAsync(client, clientSecret, cancellationToken);
            }
        }

        // The generation of the very record the secret was checked against.
        return ok ? client!.Generation : null;
    }

    /// <summary>Swaps an older hash format for the current one, now that the secret is at hand. The client stays valid if this fails.</summary>
    private async Task UpgradeHashAsync(RegisteredClient client, string secret, CancellationToken cancellationToken)
    {
        try
        {
            var upgraded = Secrets.Hash(secret);
            await _file.UpdateAsync(current => current with
            {
                Clients = [.. current.Clients.Select(c => c.Id == client.Id && c.SecretHash == client.SecretHash ? c with { SecretHash = upgraded } : c)],
            }, cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            // Not fatal: the old hash still verifies, and the next token request tries again.
        }
    }

    public void Dispose()
    {
        _file.Dispose();
        _certificate?.Dispose();
    }

    private IReadOnlyList<ClientView> ToViews(ServerSecrets secrets) => [.. secrets.Clients.Select(ToView)];

    private ClientView ToView(RegisteredClient c) =>
        new(c.Id, c.Name, c.CreatedAt, c.Secret is not null, _lastSeen.TryGetValue(c.Id, out var seen) ? seen : null);

    private static string NewSigningKey()
    {
        using var rsa = RSA.Create(2048);
        return Convert.ToBase64String(rsa.ExportPkcs8PrivateKey());
    }
}
