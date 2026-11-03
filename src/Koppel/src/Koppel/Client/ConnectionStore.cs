using System.Security.Cryptography.X509Certificates;
using Koppel.Core.Configuration;
using Koppel.Core.Security;

namespace Koppel.Client;

/// <summary>The contents of the encrypted <c>connection.json</c> (client role).</summary>
/// <param name="ServerFingerprint">The certificate pinned for this server.</param>
/// <param name="NextServerFingerprint">
/// The certificate the server said it will switch to, trusted as well until the switch. It only ever comes
/// from the server itself, over a link pinned to a certificate this connection already trusts.
/// </param>
internal sealed record ClientConnection(string? ServerUrl, string? ServerFingerprint, string? ClientId, string? ClientSecret, string? NextServerFingerprint = null)
{
    public static ClientConnection Empty { get; } = new(null, null, null, null);

    public bool IsComplete =>
        ServerUrl is not null && ServerFingerprint is not null && ClientId is not null && ClientSecret is not null;

    /// <summary>The server may present this certificate: it is the pinned one, or the next one the server announced.</summary>
    public bool Trusts(X509Certificate2? certificate) => ServerCertificate.MatchesAny(certificate, ServerFingerprint, NextServerFingerprint);

    /// <summary>
    /// This is <paramref name="previous"/>'s server, at most moved to another address or on to its next
    /// certificate: the same credentials, pinned to the same fingerprint or to the next one
    /// <paramref name="previous"/> was told about. Such a change keeps the attached devices, and the link
    /// too while the address stays; any other is a new server.
    /// </summary>
    public bool Continues(ClientConnection previous) =>
        ClientId == previous.ClientId
        && ClientSecret == previous.ClientSecret
        && ServerFingerprint is not null
        && (ConnectionUpdate.SameFingerprint(ServerFingerprint, previous.ServerFingerprint) || ConnectionUpdate.SameFingerprint(ServerFingerprint, previous.NextServerFingerprint));

    /// <summary>
    /// What the server said about its certificates, over a link pinned to one this connection trusts. When it
    /// now presents the next one, that becomes the pinned one; either way the next one is what it says, or
    /// none. A statement about a certificate this connection does not know changes nothing.
    /// </summary>
    public ClientConnection WithServerCertificates(string fingerprint, string? next)
    {
        var current = ConnectionUpdate.SameFingerprint(fingerprint, ServerFingerprint) ? ServerFingerprint
            : ConnectionUpdate.SameFingerprint(fingerprint, NextServerFingerprint) ? NextServerFingerprint
            : null;
        if (current is null)
        {
            return this;
        }

        return this with
        {
            ServerFingerprint = current,
            NextServerFingerprint = next is null || ConnectionUpdate.SameFingerprint(next, current) ? null : ConnectionUpdate.Normalize(next),
        };
    }
}

/// <summary>What the UI may see about the connection: never the secret, only whether one is stored.</summary>
/// <param name="NextServerFingerprint">The certificate the server will switch to, which this client trusts as well until then.</param>
public sealed record ConnectionView(string? ServerUrl, string? ServerFingerprint, string? ClientId, bool HasSecret, string? NextServerFingerprint = null);

/// <summary>Owns <c>connection.json</c>. Same write-then-raise rule as settings.</summary>
internal sealed class ConnectionStore : IHostedService, IDisposable
{
    private readonly JsonConfigFile<ClientConnection> _file;

    public ConnectionStore(KoppelPaths paths, SecretBox box, ILogger<ConnectionStore> logger)
    {
        _file = new JsonConfigFile<ClientConnection>(paths.ConnectionFile, ClientConnection.Empty, KoppelJson.Files, logger, box.AsFileCodec());
        _file.Changed += connection => Changed.RaiseAsync(ToView(connection), logger);
    }

    public event Func<ConnectionView, Task>? Changed;

    public ClientConnection Current => _file.Current;

    public ConnectionView View => ToView(_file.Current);

    public Task StartAsync(CancellationToken cancellationToken) => _file.StartAsync(cancellationToken);

    public Task StopAsync(CancellationToken cancellationToken)
    {
        _file.Stop();
        return Task.CompletedTask;
    }

    public Task<ClientConnection> UpdateAsync(Func<ClientConnection, ClientConnection> change, CancellationToken cancellationToken) =>
        _file.UpdateAsync(change, cancellationToken);

    /// <summary>
    /// Stores what the server said about its certificates (see <see cref="ClientConnection.WithServerCertificates"/>),
    /// as long as the saved connection is still the one <paramref name="link"/> was made for. Writes nothing
    /// when that changes nothing, so the same message on every reconnect costs no write and no push.
    /// </summary>
    public async Task<ClientConnection> LearnCertificatesAsync(ClientConnection link, string fingerprint, string? next, CancellationToken cancellationToken)
    {
        var current = _file.Current;
        if (!current.Continues(link) || current.WithServerCertificates(fingerprint, next) == current)
        {
            return current;
        }

        return await _file.UpdateAsync(c => c.Continues(link) ? c.WithServerCertificates(fingerprint, next) : c, cancellationToken);
    }

    /// <summary>
    /// Points the saved connection at <paramref name="serverUrl"/>, where its server moved to, as long as the
    /// saved connection is still the one <paramref name="link"/> was made for: whatever the person saved
    /// meanwhile wins. Only the address changes. True when it moved.
    /// </summary>
    public async Task<bool> MoveServerAsync(ClientConnection link, string serverUrl, CancellationToken cancellationToken)
    {
        if (_file.Current != link)
        {
            return false;
        }

        var moved = link with { ServerUrl = serverUrl };
        return await _file.UpdateAsync(c => c == link ? moved : c, cancellationToken) == moved;
    }

    public void Dispose() => _file.Dispose();

    private static ConnectionView ToView(ClientConnection c) =>
        new(c.ServerUrl, c.ServerFingerprint, c.ClientId, c.ClientSecret is not null, c.NextServerFingerprint);
}
