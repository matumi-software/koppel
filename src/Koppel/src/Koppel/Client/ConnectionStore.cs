using Koppel.Core.Configuration;
using Koppel.Core.Security;
using Skakel.Client;

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

    // Methods, not properties: a record prints its properties, and the config file would save them.

    /// <summary>The server's address, for Skakel. Only for a complete connection.</summary>
    public Uri GetAddress() => new(ServerUrl!);

    /// <summary>The certificates the server may present: the pinned one, and the next one it announced.</summary>
    public SkakelPins GetPins() => new(ServerFingerprint!, NextServerFingerprint);

    public SkakelCredentials GetCredentials() => new(ClientId!, ClientSecret!);

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
    /// Stores the certificates the link to <paramref name="link"/>'s server trusts now, after the server told
    /// it of a next one, switched to it or cancelled it, as long as the saved connection is still that server.
    /// Writes nothing when that changes nothing. Throws when the saved connection is another server's, so the
    /// link does not tell the server it has the next certificate.
    /// </summary>
    public async Task SavePinsAsync(ClientConnection link, SkakelPins pins, CancellationToken cancellationToken)
    {
        ClientConnection With(ClientConnection c) => c with { ServerFingerprint = pins.Fingerprint, NextServerFingerprint = pins.NextFingerprint };
        var current = _file.Current;
        if (!current.Continues(link))
        {
            throw new InvalidOperationException("The saved connection is for another server now.");
        }

        if (With(current) != current)
        {
            await _file.UpdateAsync(c => c.Continues(link) ? With(c) : c, cancellationToken);
        }
    }

    /// <summary>
    /// Points the saved connection at <paramref name="serverUrl"/>, where its server moved to, as long as the
    /// saved connection is still <paramref name="link"/>'s server at <paramref name="link"/>'s address:
    /// whatever the person saved meanwhile wins. Only the address changes. True when it moved.
    /// </summary>
    public async Task<bool> MoveServerAsync(ClientConnection link, string serverUrl, CancellationToken cancellationToken)
    {
        bool Same(ClientConnection c) => c.ServerUrl == link.ServerUrl && c.Continues(link);
        if (!Same(_file.Current))
        {
            return false;
        }

        var written = await _file.UpdateAsync(c => Same(c) ? c with { ServerUrl = serverUrl } : c, cancellationToken);
        return written.ServerUrl == serverUrl && written.Continues(link);
    }

    public void Dispose() => _file.Dispose();

    private static ConnectionView ToView(ClientConnection c) =>
        new(c.ServerUrl, c.ServerFingerprint, c.ClientId, c.ClientSecret is not null, c.NextServerFingerprint);
}
