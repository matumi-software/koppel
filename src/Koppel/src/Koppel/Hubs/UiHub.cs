using Koppel.Client;
using Koppel.Server;
using Koppel.Settings;
using Koppel.Usb;
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

    /// <summary>Server role: this machine's devices after a share change, a plug or an attach.</summary>
    Task DevicesChanged(ServerDevices devices);

    /// <summary>Client role: the server's shared devices and what is attached here.</summary>
    Task ClientDevicesChanged(ClientDevices devices);
}

/// <summary>The UI's live link, mapped at <c>/api/hubs/ui</c>. Push only; commands go over REST.</summary>
internal sealed class UiHub : Hub<IUiClient>;

/// <summary>Forwards every confirmed change from the stores to all connected UIs.</summary>
internal sealed class UiHubRelay(
    SettingsStore settings,
    ServerSecretsStore secrets,
    ConnectionStore connection,
    UsbServer usbServer,
    UsbClient usbClient,
    IHubContext<UiHub, IUiClient> hub,
    IHubContext<RemoteHub, IRemoteClient> remote) : IHostedService
{
    public Task StartAsync(CancellationToken cancellationToken)
    {
        settings.Changed += PushSettingsAsync;
        secrets.ClientsChanged += PushClientsAsync;
        connection.Changed += PushConnectionAsync;
        usbServer.Changed += PushServerDevicesAsync;
        usbClient.Changed += PushClientDevicesAsync;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= PushSettingsAsync;
        secrets.ClientsChanged -= PushClientsAsync;
        connection.Changed -= PushConnectionAsync;
        usbServer.Changed -= PushServerDevicesAsync;
        usbClient.Changed -= PushClientDevicesAsync;
        return Task.CompletedTask;
    }

    private Task PushSettingsAsync(KoppelSettings value) => hub.Clients.All.SettingsChanged(value);

    private Task PushClientsAsync(IReadOnlyList<ClientView> value) => hub.Clients.All.ClientsChanged(value);

    private Task PushConnectionAsync(ConnectionView value) => hub.Clients.All.ConnectionChanged(value);

    private async Task PushServerDevicesAsync(ServerDevices value)
    {
        await hub.Clients.All.DevicesChanged(value);
        await remote.Clients.All.DevicesChanged(value.ForRemote(settings.Current.ListenPort));
    }

    private Task PushClientDevicesAsync(ClientDevices value) => hub.Clients.All.ClientDevicesChanged(value);
}
