using System.Collections.Concurrent;
using Koppel.Server;

namespace Koppel.Usb;

/// <summary>
/// The live sessions of remote clients: USB connections and remote hub connections. When the client
/// list changes and a client is gone, or its credentials were rotated, its sessions are aborted, so
/// revoking a client ends what it is doing now and not only what it asks for next.
/// </summary>
internal sealed partial class LiveSessions(ServerSecretsStore secrets, ILogger<LiveSessions> logger) : IHostedService
{
    private readonly ConcurrentDictionary<long, Session> _sessions = new();
    private long _next;

    public Task StartAsync(CancellationToken cancellationToken)
    {
        secrets.ClientsChanged += OnClientsChanged;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        secrets.ClientsChanged -= OnClientsChanged;
        return Task.CompletedTask;
    }

    /// <summary>
    /// Tracks a session until the returned handle is disposed. <paramref name="abort"/> ends it. A session
    /// whose client is already out of date is aborted at once, which closes the gap between the check
    /// that let it in and this call.
    /// </summary>
    public IDisposable Add(string clientId, int generation, Action abort, string? connectionId = null)
    {
        var id = Interlocked.Increment(ref _next);
        var session = new Session(clientId, generation, abort, connectionId);
        _sessions[id] = session;
        if (!secrets.IsCurrent(clientId, generation))
        {
            Abort(session);
        }

        return new Handle(() => _sessions.TryRemove(id, out _));
    }

    /// <summary>The remote hub connections of clients that are still registered at their current generation.</summary>
    public IReadOnlyList<string> RemoteConnectionIds() =>
        [.. _sessions.Values
            .Where(s => s.ConnectionId is not null && secrets.IsCurrent(s.ClientId, s.Generation))
            .Select(s => s.ConnectionId!)];

    private Task OnClientsChanged(IReadOnlyList<ClientView> clients)
    {
        foreach (var session in _sessions.Values.Where(s => !secrets.IsCurrent(s.ClientId, s.Generation)))
        {
            Abort(session);
        }

        return Task.CompletedTask;
    }

    private void Abort(Session session)
    {
        LogAborted(session.ClientId);
        try
        {
            session.Abort();
        }
        catch (Exception ex) when (ex is ObjectDisposedException or InvalidOperationException)
        {
            // The session ended by itself first.
        }
    }

    [LoggerMessage(Level = LogLevel.Information, Message = "Ending a session of client {ClientId}: it was revoked or its secret was rotated")]
    private partial void LogAborted(string clientId);

    private sealed record Session(string ClientId, int Generation, Action Abort, string? ConnectionId);

    private sealed class Handle(Action release) : IDisposable
    {
        public void Dispose() => release();
    }
}
