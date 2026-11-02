using Koppel.Settings;
using Microsoft.AspNetCore.SignalR;

namespace Koppel.Hubs;

/// <summary>Messages the server pushes to the UI.</summary>
/// <remarks>Public, with the types it carries, because SignalR emits its proxy in another assembly.</remarks>
public interface IUiClient
{
    /// <summary>The settings as written to disk. The UI treats a change as done only when this arrives.</summary>
    Task SettingsChanged(KoppelSettings settings);
}

/// <summary>The UI's live link, mapped at <c>/api/hubs/ui</c>. Push only; commands go over REST.</summary>
internal sealed class UiHub : Hub<IUiClient>;

/// <summary>Forwards every confirmed settings change to all connected UIs.</summary>
internal sealed class SettingsHubRelay(SettingsStore store, IHubContext<UiHub, IUiClient> hub) : IHostedService
{
    public Task StartAsync(CancellationToken cancellationToken)
    {
        store.Changed += PushAsync;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        store.Changed -= PushAsync;
        return Task.CompletedTask;
    }

    private Task PushAsync(KoppelSettings settings) => hub.Clients.All.SettingsChanged(settings);
}
