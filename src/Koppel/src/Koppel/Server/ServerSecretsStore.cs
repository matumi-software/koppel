using System.Security.Claims;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using Koppel.Core.Auth;
using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Microsoft.Extensions.Primitives;
using Microsoft.IdentityModel.Tokens;

namespace Koppel.Server;

/// <summary>
/// A client allowed to use this server. The secret is kept, inside the encrypted file, so an admin can
/// copy it again later. Clients made before that have only the hash.
/// </summary>
internal sealed record RegisteredClient(string Id, string Name, string SecretHash, DateTimeOffset CreatedAt, string? Secret = null, int Generation = 1);

/// <summary>A token signing key that no longer signs, kept so the tokens it signed stay valid until they expire.</summary>
internal sealed record RetiredSigningKey(string Key, string KeyId, DateTimeOffset AcceptUntil);

/// <summary>
/// The certificate the server presents after the switch, kept beside the current one while the paired
/// clients learn its fingerprint.
/// </summary>
/// <param name="ReceivedBy">The clients that said they stored its fingerprint.</param>
internal sealed record NextCertificate(string Pfx, DateTimeOffset PreparedAt, IReadOnlyList<string> ReceivedBy);

/// <summary>The contents of the encrypted <c>secrets.json</c> (server role).</summary>
/// <param name="SigningKeyId">Null for the key made at first start before keys could be rotated, which is <c>koppel-1</c>.</param>
/// <param name="SigningKeySince">When a rotation made the signing key; null for the first one.</param>
internal sealed record ServerSecrets(
    string? SigningKey,
    string? CertificatePfx,
    IReadOnlyList<RegisteredClient> Clients,
    string? SigningKeyId = null,
    DateTimeOffset? SigningKeySince = null,
    IReadOnlyList<RetiredSigningKey>? RetiredSigningKeys = null,
    NextCertificate? NextCertificate = null)
{
    public static ServerSecrets Empty { get; } = new(null, null, []);
}

/// <summary>What the UI lists about a client. The secret is fetched one client at a time, on demand.</summary>
/// <param name="HasSecret">False for clients made before secrets were kept: rotate to get a copyable one.</param>
/// <param name="LastSeen">When the client last got a token, since this server started.</param>
/// <param name="HasNextCertificate">The client stored the fingerprint of the prepared next certificate.</param>
public sealed record ClientView(string Id, string Name, DateTimeOffset CreatedAt, bool HasSecret, DateTimeOffset? LastSeen, bool HasNextCertificate = false);

/// <summary>What the UI shows about the server's certificate and its token signing key.</summary>
/// <param name="NextFingerprint">The prepared next certificate's fingerprint, or null when none is prepared.</param>
/// <param name="SigningKeySince">When a rotation made the signing key; null for the one made at first start.</param>
/// <param name="RetiredKeysUntil">While keys it replaced are still accepted, when the last of them is dropped.</param>
public sealed record ServerKeysView(string Fingerprint, string? NextFingerprint, DateTimeOffset? NextPreparedAt, string SigningKeyId, DateTimeOffset? SigningKeySince, DateTimeOffset? RetiredKeysUntil);

/// <summary>What a switch to the next certificate did.</summary>
/// <param name="NotReceived">The names of the clients that had not stored the new fingerprint.</param>
internal sealed record CertificateSwitch(string From, string To, IReadOnlyList<string> NotReceived);

/// <summary>
/// Owns <c>secrets.json</c>: the token signing key, the HTTPS certificate and the registered clients.
/// Creates the key and certificate on first start. Same write-then-raise rule as settings.
/// </summary>
/// <remarks>
/// Both can be rotated. A new signing key signs from then on, and the one it replaced is still accepted
/// until every token it signed has expired (<see cref="RetiredKeyMargin"/> past the token lifetime), then
/// dropped. A new certificate is prepared first and kept beside the current one, so paired clients can
/// learn its fingerprint over the link they already trust, and the switch presents it.
/// </remarks>
internal sealed partial class ServerSecretsStore : IHostedService, IClientCredentialValidator, ISigningKeySource, IDisposable
{
    /// <summary>The id of the key made at first start, before keys had ids of their own.</summary>
    private const string FirstKeyId = "koppel-1";

    /// <summary>How long past the token lifetime a replaced key is still accepted, for a token issued while the rotation was applied.</summary>
    internal static readonly TimeSpan RetiredKeyMargin = TimeSpan.FromMinutes(5);

    private readonly JsonConfigFile<ServerSecrets> _file;
    private readonly TimeProvider _time;
    private readonly ILogger<ServerSecretsStore> _logger;
    private readonly System.Collections.Concurrent.ConcurrentDictionary<string, DateTimeOffset> _lastSeen = new();
    private readonly Lock _applying = new();
    private readonly Dictionary<string, RsaSecurityKey> _rsaKeys = [];
    private readonly List<X509Certificate2> _replaced = [];
    private readonly ITimer _prune;
    private SigningKeys? _keys;
    private string? _keysSource;
    private CancellationTokenSource _keysChanged = new();
    private X509Certificate2? _certificate;
    private string? _certificatePfx;
    private string? _nextPfx;
    private string? _nextFingerprint;
    private ServerKeysView? _raisedKeys;

    public ServerSecretsStore(KoppelPaths paths, SecretBox box, TimeProvider time, ILogger<ServerSecretsStore> logger)
    {
        _time = time;
        _logger = logger;
        _file = new JsonConfigFile<ServerSecrets>(paths.ServerSecretsFile, ServerSecrets.Empty, KoppelJson.Files, logger, box.AsFileCodec());
        _file.Changed += OnFileChangedAsync;
        _prune = time.CreateTimer(_ => _ = PruneRetiredKeysAsync(), null, Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
    }

    /// <summary>Raised after the client list on disk changed.</summary>
    public event Func<IReadOnlyList<ClientView>, Task>? ClientsChanged;

    /// <summary>Raised after the certificate or the signing key changed, with what the UI shows of them.</summary>
    public event Func<ServerKeysView, Task>? KeysChanged;

    public IReadOnlyList<ClientView> Clients => ToViews(_file.Current);

    public SecurityKey SigningKey => Keys.Signing;

    public IReadOnlyList<SecurityKey> ValidationKeys => Keys.Validation;

    public IChangeToken GetChangeToken() => new CancellationChangeToken(Volatile.Read(ref _keysChanged).Token);

    /// <summary>The certificate this server presents. After a switch it is the new one, at once, for every new connection.</summary>
    public X509Certificate2 Certificate => Volatile.Read(ref _certificate) ?? throw new InvalidOperationException("Server secrets are not loaded yet.");

    public string Fingerprint => ServerCertificate.Fingerprint(Certificate);

    /// <summary>The prepared next certificate's fingerprint, or null when none is prepared.</summary>
    public string? NextFingerprint => Volatile.Read(ref _nextFingerprint);

    /// <summary>What the UI shows about the certificate and the signing key.</summary>
    public ServerKeysView KeysView
    {
        get
        {
            var secrets = _file.Current;
            return new ServerKeysView(
                Fingerprint,
                NextFingerprint,
                secrets.NextCertificate?.PreparedAt,
                secrets.SigningKeyId ?? FirstKeyId,
                secrets.SigningKeySince,
                secrets.RetiredSigningKeys is { Count: > 0 } retired ? retired.Max(r => r.AcceptUntil) : null);
        }
    }

    private SigningKeys Keys => Volatile.Read(ref _keys) ?? throw new InvalidOperationException("Server secrets are not loaded yet.");

    public async Task StartAsync(CancellationToken cancellationToken)
    {
        await _file.StartAsync(cancellationToken);
        var secrets = _file.Current;
        var now = _time.GetUtcNow();
        if (secrets.SigningKey is null || secrets.CertificatePfx is null || secrets.RetiredSigningKeys?.Any(r => r.AcceptUntil <= now) == true)
        {
            secrets = await _file.UpdateAsync(current => current with
            {
                SigningKey = current.SigningKey ?? NewSigningKey(),
                CertificatePfx = current.CertificatePfx ?? Convert.ToBase64String(ServerCertificate.CreatePfx(Environment.MachineName)),
                RetiredSigningKeys = Unexpired(current.RetiredSigningKeys, now),
            }, cancellationToken);
        }

        Apply(secrets);
        _raisedKeys = KeysView;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        _file.Stop();
        _prune.Change(Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
        return Task.CompletedTask;
    }

    /// <summary>
    /// Makes a new signing key and signs with it from now on. The one it replaces is still accepted until
    /// the tokens it signed have expired, so no client notices. Returns the new key's id.
    /// </summary>
    public async Task<string> RotateSigningKeyAsync(CancellationToken cancellationToken)
    {
        var key = NewSigningKey();
        var id = Secrets.NewId("koppel");
        var now = _time.GetUtcNow();
        await _file.UpdateAsync(current => current with
        {
            SigningKey = key,
            SigningKeyId = id,
            SigningKeySince = now,
            RetiredSigningKeys =
            [
                .. Unexpired(current.RetiredSigningKeys, now) ?? [],
                new RetiredSigningKey(current.SigningKey!, current.SigningKeyId ?? FirstKeyId, now + KoppelAuthServer.AccessTokenLifetime + RetiredKeyMargin),
            ],
        }, cancellationToken);
        return id;
    }

    /// <summary>
    /// Makes the next certificate and keeps it beside the current one, which the server goes on presenting.
    /// Returns its fingerprint, or null when one is already prepared.
    /// </summary>
    public async Task<string?> PrepareCertificateAsync(CancellationToken cancellationToken)
    {
        if (_file.Current.NextCertificate is not null)
        {
            return null;
        }

        var pfx = Convert.ToBase64String(ServerCertificate.CreatePfx(Environment.MachineName));
        var written = await _file.UpdateAsync(current => current.NextCertificate is null
            ? current with { NextCertificate = new NextCertificate(pfx, _time.GetUtcNow(), []) }
            : current, cancellationToken);
        return written.NextCertificate?.Pfx == pfx ? NextFingerprint : null;
    }

    /// <summary>Drops the prepared next certificate; clients that stored its fingerprint are told to forget it. False when none was prepared.</summary>
    public async Task<bool> CancelCertificateAsync(CancellationToken cancellationToken)
    {
        if (_file.Current.NextCertificate is null)
        {
            return false;
        }

        await _file.UpdateAsync(current => current with { NextCertificate = null }, cancellationToken);
        return true;
    }

    /// <summary>
    /// Presents the prepared certificate from now on; the old one is gone. Connections already open carry on
    /// over the old one. Returns what changed, or null when no certificate was prepared.
    /// </summary>
    public async Task<CertificateSwitch?> SwitchCertificateAsync(CancellationToken cancellationToken)
    {
        var from = Fingerprint;
        NextCertificate? next = null;
        IReadOnlyList<RegisteredClient> clients = [];
        await _file.UpdateAsync(current =>
        {
            next = current.NextCertificate;
            clients = current.Clients;
            return next is null ? current : current with { CertificatePfx = next.Pfx, NextCertificate = null };
        }, cancellationToken);

        return next is not { } switched
            ? null
            : new CertificateSwitch(from, Fingerprint, [.. clients.Where(c => !switched.ReceivedBy.Contains(c.Id)).Select(c => c.Name)]);
    }

    /// <summary>
    /// Records that a client stored the next certificate's fingerprint, so the UI can say when it is safe to
    /// switch. Ignored unless <paramref name="fingerprint"/> is the prepared one and the client is registered.
    /// </summary>
    public async Task MarkNextCertificateReceivedAsync(string clientId, string fingerprint, CancellationToken cancellationToken)
    {
        var next = _file.Current.NextCertificate;
        if (next is null || next.ReceivedBy.Contains(clientId) || !IsRegistered(clientId) || !SameFingerprint(fingerprint, NextFingerprint))
        {
            return;
        }

        await _file.UpdateAsync(current => current.NextCertificate is { } n && n.Pfx == next.Pfx && !n.ReceivedBy.Contains(clientId)
            ? current with { NextCertificate = n with { ReceivedBy = [.. n.ReceivedBy, clientId] } }
            : current, cancellationToken);
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

        await _file.UpdateAsync(current => current with
        {
            Clients = [.. current.Clients.Where(c => c.Id != clientId)],
            NextCertificate = current.NextCertificate is { } next ? next with { ReceivedBy = [.. next.ReceivedBy.Where(id => id != clientId)] } : null,
        }, cancellationToken);
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
        _prune.Dispose();
        _file.Dispose();
        lock (_applying)
        {
            _certificate?.Dispose();
            foreach (var certificate in _replaced)
            {
                certificate.Dispose();
            }
        }
    }

    /// <summary>Every write and hand edit lands here: the keys and certificates in use follow the file, then the UI hears of it.</summary>
    private async Task OnFileChangedAsync(ServerSecrets secrets)
    {
        Apply(secrets);
        await ClientsChanged.RaiseAsync(ToViews(secrets), _logger);
        var keys = KeysView;
        if (keys != _raisedKeys)
        {
            _raisedKeys = keys;
            await KeysChanged.RaiseAsync(keys, _logger);
        }
    }

    /// <summary>
    /// Loads what changed: the signing keys (and tells the token server and the validation), the certificate
    /// presented, and the next certificate's fingerprint. A replaced certificate is kept until the store is
    /// disposed, because a TLS handshake may still be using it.
    /// </summary>
    private void Apply(ServerSecrets secrets)
    {
        if (secrets.SigningKey is null || secrets.CertificatePfx is null)
        {
            return;
        }

        var now = _time.GetUtcNow();
        var retired = Unexpired(secrets.RetiredSigningKeys, now) ?? [];
        var keysChanged = false;
        lock (_applying)
        {
            var source = string.Join('|', [secrets.SigningKey, secrets.SigningKeyId, .. retired.Select(r => r.KeyId)]);
            if (source != _keysSource)
            {
                var signing = RsaKey(secrets.SigningKey, secrets.SigningKeyId ?? FirstKeyId);
                var validation = new List<SecurityKey> { signing };
                validation.AddRange(retired.Select(r => RsaKey(r.Key, r.KeyId)));
                foreach (var stale in _rsaKeys.Keys.Except(validation.Select(k => k.KeyId)).ToList())
                {
                    _rsaKeys.Remove(stale);
                }

                Volatile.Write(ref _keys, new SigningKeys(signing, validation));
                _keysSource = source;
                keysChanged = true;
            }

            if (secrets.CertificatePfx != _certificatePfx)
            {
                var previous = _certificate;
                Volatile.Write(ref _certificate, ServerCertificate.Load(Convert.FromBase64String(secrets.CertificatePfx)));
                _certificatePfx = secrets.CertificatePfx;
                if (previous is not null)
                {
                    _replaced.Add(previous);
                }
            }

            var nextPfx = secrets.NextCertificate?.Pfx;
            if (nextPfx != _nextPfx)
            {
                Volatile.Write(ref _nextFingerprint, nextPfx is null ? null : FingerprintOf(nextPfx));
                _nextPfx = nextPfx;
            }
        }

        if (keysChanged)
        {
            try
            {
                // The token server and the validation build their options again, with these keys, before this returns.
                Interlocked.Exchange(ref _keysChanged, new CancellationTokenSource()).Cancel();
            }
            catch (AggregateException ex)
            {
                LogKeysNotApplied(ex);
            }
        }

        // Wake up when the first replaced key is due to go.
        _prune.Change(retired.Count > 0 ? Max(retired.Min(r => r.AcceptUntil) - now, TimeSpan.Zero) : Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
    }

    /// <summary>Drops the replaced keys whose tokens have all expired. A failure is logged and tried again in a minute.</summary>
    private async Task PruneRetiredKeysAsync()
    {
        try
        {
            var now = _time.GetUtcNow();
            await _file.UpdateAsync(current => current with { RetiredSigningKeys = Unexpired(current.RetiredSigningKeys, now) }, CancellationToken.None);
        }
        catch (Exception ex)
        {
            LogPruneFailed(ex);
            _prune.Change(TimeSpan.FromMinutes(1), Timeout.InfiniteTimeSpan);
        }
    }

    private RsaSecurityKey RsaKey(string pkcs8, string keyId)
    {
        if (!_rsaKeys.TryGetValue(keyId, out var key))
        {
            var rsa = RSA.Create();
            rsa.ImportPkcs8PrivateKey(Convert.FromBase64String(pkcs8), out _);
            key = new RsaSecurityKey(rsa) { KeyId = keyId };
            _rsaKeys[keyId] = key;
        }

        return key;
    }

    private static List<RetiredSigningKey>? Unexpired(IReadOnlyList<RetiredSigningKey>? retired, DateTimeOffset now) =>
        retired?.Where(r => r.AcceptUntil > now).ToList() is { Count: > 0 } kept ? kept : null;

    private static TimeSpan Max(TimeSpan a, TimeSpan b) => a > b ? a : b;

    private static string FingerprintOf(string pfx)
    {
        using var certificate = ServerCertificate.Load(Convert.FromBase64String(pfx));
        return ServerCertificate.Fingerprint(certificate);
    }

    private static bool SameFingerprint(string? a, string? b) =>
        a is not null && b is not null && string.Equals(Digits(a), Digits(b), StringComparison.OrdinalIgnoreCase);

    private static string Digits(string fingerprint) => new([.. fingerprint.Where(Uri.IsHexDigit)]);

    private IReadOnlyList<ClientView> ToViews(ServerSecrets secrets) => [.. secrets.Clients.Select(c => ToView(c, secrets.NextCertificate))];

    private ClientView ToView(RegisteredClient c, NextCertificate? next = null) =>
        new(c.Id, c.Name, c.CreatedAt, c.Secret is not null, _lastSeen.TryGetValue(c.Id, out var seen) ? seen : null, next?.ReceivedBy.Contains(c.Id) == true);

    private static string NewSigningKey()
    {
        using var rsa = RSA.Create(2048);
        return Convert.ToBase64String(rsa.ExportPkcs8PrivateKey());
    }

    /// <summary>The key that signs, and every key a live token may carry, the signing key first.</summary>
    private sealed record SigningKeys(RsaSecurityKey Signing, IReadOnlyList<SecurityKey> Validation);

    [LoggerMessage(Level = LogLevel.Error, Message = "The token server did not take the new signing keys")]
    private partial void LogKeysNotApplied(Exception exception);

    [LoggerMessage(Level = LogLevel.Warning, Message = "Could not drop the expired token signing keys from secrets.json; trying again in a minute")]
    private partial void LogPruneFailed(Exception exception);
}
