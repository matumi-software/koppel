using Koppel.Client;
using Koppel.Server;
using Koppel.Settings;
using Microsoft.AspNetCore.SignalR;

namespace Koppel.Hubs;

/// <summary>Messages the server pushes to the UI. Each carries the value as written to disk.</summary>
/// <remarks>Public, with the types it carries, because SignalR emits its proxy in another assembly.</remarks>
public interface IUiClient
{
    /// <summary>The UI treats a settings change as done only when this arrives.</summary>
    Task SettingsChanged(KoppelSettings settings);

    /// <summary>Server role: the registered clients after a create or revoke.</summary>
    Task ClientsChanged(IReadOnlyList<ClientView> clients);

    /// <summary>Client role: the stored server connection, without the secret.</summary>
    Task ConnectionChanged(ConnectionView connection);
}

/// <summary>The UI's live link, mapped at <c>/api/hubs/ui</c>. Push only; commands go over REST.</summary>
internal sealed class UiHub : Hub<IUiClient>;

/// <summary>Forwards every confirmed change from the stores to all connected UIs.</summary>
internal sealed class UiHubRelay(
    SettingsStore settings,
    ServerSecretsStore secrets,
    ConnectionStore connection,
    IHubContext<UiHub, IUiClient> hub) : IHostedService
{
    public Task StartAsync(CancellationToken cancellationToken)
    {
        settings.Changed += PushSettingsAsync;
        secrets.ClientsChanged += PushClientsAsync;
        connection.Changed += PushConnectionAsync;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= PushSettingsAsync;
        secrets.ClientsChanged -= PushClientsAsync;
        connection.Changed -= PushConnectionAsync;
        return Task.CompletedTask;
    }

    private Task PushSettingsAsync(KoppelSettings value) => hub.Clients.All.SettingsChanged(value);

    private Task PushClientsAsync(IReadOnlyList<ClientView> value) => hub.Clients.All.ClientsChanged(value);

    private Task PushConnectionAsync(ConnectionView value) => hub.Clients.All.ConnectionChanged(value);
}
