using System.Collections.Concurrent;
using Microsoft.AspNetCore.SignalR;

namespace Koppel.Login;

/// <summary>
/// The UI hub's live connections. A cookie is only checked when a request starts, and a hub connection
/// is one long request, so ending a session has to close its connections too.
/// </summary>
internal sealed class UiConnections(UiAuthStore store) : IHostedService
{
    private readonly ConcurrentDictionary<string, HubCallerContext> _open = new();

    public void Add(HubCallerContext context) => _open[context.ConnectionId] = context;

    public void Remove(string connectionId) => _open.TryRemove(connectionId, out _);

    /// <summary>Closes every connection of one session, such as after Sign out.</summary>
    public void CloseSession(string? sessionId)
    {
        foreach (var context in _open.Values.Where(c => c.User?.FindFirst(UiSession.SessionClaim)?.Value == sessionId))
        {
            context.Abort();
        }
    }

    public Task StartAsync(CancellationToken cancellationToken)
    {
        store.StampChanged += CloseStaleAsync;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        store.StampChanged -= CloseStaleAsync;
        return Task.CompletedTask;
    }

    private Task CloseStaleAsync(UiStampChange change)
    {
        foreach (var context in _open.Values.Where(c => c.User?.FindFirst(UiSession.SessionClaim)?.Value != change.KeepSession))
        {
            context.Abort();
        }

        return Task.CompletedTask;
    }
}
