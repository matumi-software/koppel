using System.Security.Cryptography;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Skakel.Server;

namespace Koppel.Server;

/// <summary>A client as Koppel 0.1.0 kept it. Read once, to carry a server over to Skakel.</summary>
internal sealed record LegacyClient(string Id, string Name, string SecretHash, DateTimeOffset CreatedAt, string? Secret = null, int Generation = 1);

/// <summary>A prepared next certificate as Koppel 0.1.0 kept it.</summary>
internal sealed record LegacyNextCertificate(string Pfx, DateTimeOffset PreparedAt, IReadOnlyList<string> ReceivedBy);

/// <summary>
/// The contents of the encrypted <c>secrets.json</c>: Skakel's state, which holds the certificate, the
/// token signing keys and the registered clients. The other fields are what Koppel 0.1.0 wrote there;
/// they are read once, carried over into <see cref="Skakel"/>, and not written again.
/// </summary>
internal sealed record ServerSecrets(
    SkakelState? Skakel = null,
    string? SigningKey = null,
    string? CertificatePfx = null,
    IReadOnlyList<LegacyClient>? Clients = null,
    string? SigningKeyId = null,
    DateTimeOffset? SigningKeySince = null,
    LegacyNextCertificate? NextCertificate = null)
{
    public static ServerSecrets Empty { get; } = new();
}

/// <summary>
/// Keeps Skakel's server state in <c>secrets.json</c>, encrypted like Koppel's other secret files. Skakel
/// loads it once at start and saves the whole state after every change. Every machine has one, whatever
/// its role, because the certificate is made at first start.
/// </summary>
/// <remarks>
/// A file from Koppel 0.1.0 is carried over on the first start: the certificate, the signing key and the
/// clients whose secret hash Skakel reads, so a paired client keeps its pairing once both ends run 0.1.1.
/// A file that cannot be carried over is kept beside the new one, and the server starts fresh.
/// </remarks>
internal sealed partial class ServerSecretsStore : ISkakelStore, IDisposable
{
    /// <summary>The id of the key Koppel 0.1.0 made at first start, before keys had ids of their own.</summary>
    private const string FirstKeyId = "koppel-1";

    /// <summary>The only secret hash Skakel verifies. Koppel 0.1.0 still read an older PBKDF2 one.</summary>
    private const string HashPrefix = "sha256$";

    private readonly JsonConfigFile<ServerSecrets> _file;
    private readonly TimeProvider _time;
    private readonly ILogger<ServerSecretsStore> _logger;

    public ServerSecretsStore(KoppelPaths paths, SecretBox box, TimeProvider time, ILogger<ServerSecretsStore> logger)
    {
        _time = time;
        _logger = logger;
        _file = new JsonConfigFile<ServerSecrets>(paths.ServerSecretsFile, ServerSecrets.Empty, KoppelJson.Files, logger, box.AsFileCodec());
    }

    /// <summary>
    /// Reads the file. One that cannot be read or decrypted stops the start, as every encrypted file does,
    /// and is left as it was. Null on the first start, when Skakel makes a new state and saves it.
    /// </summary>
    public async ValueTask<SkakelState?> LoadAsync(CancellationToken cancellationToken)
    {
        await _file.StartAsync(cancellationToken);
        var secrets = _file.Current;
        if (secrets.Skakel is { } state)
        {
            return state;
        }

        if (secrets.SigningKey is null || secrets.CertificatePfx is null)
        {
            return null;
        }

        if (CarryOver(secrets) is not { } carried)
        {
            var aside = $"{_file.Path}.0.1.0-{_time.GetUtcNow():yyyyMMddHHmmss}";
            File.Copy(_file.Path, aside, overwrite: true);
            LogStartedFresh(aside);
            return null;
        }

        await _file.UpdateAsync(_ => new ServerSecrets(carried), cancellationToken);
        return carried;
    }

    public async ValueTask SaveAsync(SkakelState state, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(state);
        await _file.UpdateAsync(_ => new ServerSecrets(state), cancellationToken);
    }

    public void Dispose()
    {
        _file.Stop();
        _file.Dispose();
    }

    /// <summary>
    /// Koppel 0.1.0's secrets as Skakel's state, or null when they cannot be used. The keys it retired are
    /// left behind: they only checked tokens a 0.1.1 server no longer takes.
    /// </summary>
    private SkakelState? CarryOver(ServerSecrets old)
    {
        try
        {
            // Skakel loads both when it starts; one it cannot load would stop the start.
            using (var rsa = RSA.Create())
            {
                rsa.ImportPkcs8PrivateKey(Convert.FromBase64String(old.SigningKey!), out _);
            }

            using (System.Security.Cryptography.X509Certificates.X509CertificateLoader.LoadPkcs12(Convert.FromBase64String(old.CertificatePfx!), password: null))
            {
            }

            var now = _time.GetUtcNow();
            var clients = old.Clients ?? [];
            var kept = clients.Where(c => c.SecretHash.StartsWith(HashPrefix, StringComparison.Ordinal)).ToList();
            foreach (var dropped in clients.Except(kept))
            {
                LogClientDropped(dropped.Name, dropped.Id);
            }

            var state = new SkakelState
            {
                Clients = [.. kept.Select(c => new SkakelClientRecord(c.Id, c.Name, c.SecretHash, c.CreatedAt, c.Generation, c.Secret))],
                SigningKey = new SkakelKeyRecord(old.SigningKeyId ?? FirstKeyId, old.SigningKey!, old.SigningKeySince ?? now),
                CertificatePfx = old.CertificatePfx!,
                NextCertificate = old.NextCertificate is { } next
                    ? new SkakelNextCertificateRecord(next.Pfx, next.PreparedAt, [.. next.ReceivedBy.Where(id => kept.Any(c => c.Id == id))])
                    : null,
            };
            LogCarriedOver(kept.Count);
            return state;
        }
        catch (Exception ex) when (ex is FormatException or CryptographicException or ArgumentException)
        {
            LogCannotCarryOver(ex);
            return null;
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "secrets.json was written by Koppel 0.1.0. Kept its certificate, its signing key and {Count} clients; Koppel 0.1.0 clients cannot reach this server until they run 0.1.1 too")]
    private partial void LogCarriedOver(int count);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Client {Name} ({ClientId}) was added before Koppel kept secrets in their current form and was dropped. Pair it again")]
    private partial void LogClientDropped(string name, string clientId);

    [LoggerMessage(Level = LogLevel.Warning, Message = "secrets.json was written by Koppel 0.1.0 and could not be carried over")]
    private partial void LogCannotCarryOver(Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Starting with a new certificate, signing key and client list. The 0.1.0 secrets were kept at {Path}. Every client must pair again")]
    private partial void LogStartedFresh(string path);
}
