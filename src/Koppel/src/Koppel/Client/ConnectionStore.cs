using Koppel.Core.Configuration;
using Koppel.Core.Security;

namespace Koppel.Client;

/// <summary>The contents of the encrypted <c>connection.json</c> (client role).</summary>
internal sealed record ClientConnection(string? ServerUrl, string? ServerFingerprint, string? ClientId, string? ClientSecret)
{
    public static ClientConnection Empty { get; } = new(null, null, null, null);

    public bool IsComplete =>
        ServerUrl is not null && ServerFingerprint is not null && ClientId is not null && ClientSecret is not null;
}

/// <summary>What the UI may see about the connection: never the secret, only whether one is stored.</summary>
public sealed record ConnectionView(string? ServerUrl, string? ServerFingerprint, string? ClientId, bool HasSecret);

/// <summary>Owns <c>connection.json</c>. Same write-then-raise rule as settings.</summary>
internal sealed class ConnectionStore : IHostedService, IDisposable
{
    private readonly JsonConfigFile<ClientConnection> _file;

    public ConnectionStore(KoppelPaths paths, SecretBox box, ILogger<ConnectionStore> logger)
    {
        _file = new JsonConfigFile<ClientConnection>(paths.ConnectionFile, ClientConnection.Empty, KoppelJson.Files, logger, box.AsFileCodec());
        _file.Changed += connection => Changed?.Invoke(ToView(connection)) ?? Task.CompletedTask;
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

    public void Dispose() => _file.Dispose();

    private static ConnectionView ToView(ClientConnection c) =>
        new(c.ServerUrl, c.ServerFingerprint, c.ClientId, c.ClientSecret is not null);
}
