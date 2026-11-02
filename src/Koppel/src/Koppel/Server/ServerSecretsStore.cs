using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using Koppel.Core.Auth;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Microsoft.IdentityModel.Tokens;

namespace Koppel.Server;

/// <summary>A client allowed to use this server. Only a hash of its secret is kept.</summary>
internal sealed record RegisteredClient(string Id, string Name, string SecretHash, DateTimeOffset CreatedAt);

/// <summary>The contents of the encrypted <c>secrets.json</c> (server role).</summary>
internal sealed record ServerSecrets(string? SigningKey, string? CertificatePfx, IReadOnlyList<RegisteredClient> Clients)
{
    public static ServerSecrets Empty { get; } = new(null, null, []);
}

/// <summary>What the UI may see about a client: never its secret or hash.</summary>
public sealed record ClientView(string Id, string Name, DateTimeOffset CreatedAt);

/// <summary>
/// Owns <c>secrets.json</c>: the token signing key, the HTTPS certificate and the registered clients.
/// Creates the key and certificate on first start. Same write-then-raise rule as settings.
/// </summary>
internal sealed class ServerSecretsStore : IHostedService, IClientCredentialValidator, ISigningKeySource, IDisposable
{
    private readonly JsonConfigFile<ServerSecrets> _file;
    private readonly TimeProvider _time;
    private RsaSecurityKey? _signingKey;
    private X509Certificate2? _certificate;

    public ServerSecretsStore(KoppelPaths paths, SecretBox box, TimeProvider time, ILogger<ServerSecretsStore> logger)
    {
        _time = time;
        _file = new JsonConfigFile<ServerSecrets>(paths.ServerSecretsFile, ServerSecrets.Empty, KoppelJson.Files, logger, box.AsFileCodec());
        _file.Changed += secrets => ClientsChanged?.Invoke(ToViews(secrets)) ?? Task.CompletedTask;
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

    /// <summary>Registers a client. The secret is returned once and only its hash is stored.</summary>
    public async Task<(ClientView Client, string Secret)> CreateClientAsync(string name, CancellationToken cancellationToken)
    {
        var secret = Secrets.NewSecret();
        var client = new RegisteredClient(Secrets.NewId("client"), name.Trim(), Secrets.Hash(secret), _time.GetUtcNow());
        await _file.UpdateAsync(current => current with { Clients = [.. current.Clients, client] }, cancellationToken);
        return (new ClientView(client.Id, client.Name, client.CreatedAt), secret);
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

    public ValueTask<bool> ValidateAsync(string clientId, string clientSecret, CancellationToken cancellationToken)
    {
        var client = _file.Current.Clients.FirstOrDefault(c => c.Id == clientId);
        return ValueTask.FromResult(client is not null && Secrets.Verify(clientSecret, client.SecretHash));
    }

    public void Dispose()
    {
        _file.Dispose();
        _certificate?.Dispose();
    }

    private static IReadOnlyList<ClientView> ToViews(ServerSecrets secrets) =>
        [.. secrets.Clients.Select(c => new ClientView(c.Id, c.Name, c.CreatedAt))];

    private static string NewSigningKey()
    {
        using var rsa = RSA.Create(2048);
        return Convert.ToBase64String(rsa.ExportPkcs8PrivateKey());
    }
}
