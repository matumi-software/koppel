using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Core.Discovery;
using Koppel.Discovery;
using Koppel.Login;
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

    /// <summary>Something happened. The UI prepends it to the activity log.</summary>
    Task ActivityRecorded(ActivityEvent activity);

    /// <summary>Client role: the servers found on the local network.</summary>
    Task ServersDiscovered(IReadOnlyList<DiscoveredServer> servers);

    /// <summary>Client role: where pairing stands, with the code to type on the server while it waits.</summary>
    Task PairingStatusChanged(PairingStatus status);

    /// <summary>Server role: the pairing requests waiting for someone to type their code.</summary>
    Task PairingRequestsChanged(PairingOverview pairing);
}

/// <summary>The UI's live link, mapped at <c>/api/hubs/ui</c>. Push only; commands go over REST.</summary>
internal sealed class UiHub(UiConnections connections) : Hub<IUiClient>
{
    public override Task OnConnectedAsync()
    {
        connections.Add(Context);
        return base.OnConnectedAsync();
    }

    public override Task OnDisconnectedAsync(Exception? exception)
    {
        connections.Remove(Context.ConnectionId);
        return base.OnDisconnectedAsync(exception);
    }
}

/// <summary>Forwards every confirmed change from the stores to all connected UIs.</summary>
internal sealed class UiHubRelay(
    SettingsStore settings,
    ServerSecretsStore secrets,
    ConnectionStore connection,
    UsbServer usbServer,
    UsbClient usbClient,
    UsbIpTlsServer usbListener,
    LiveSessions live,
    InMemoryActivityStore activity,
    ServerFinder finder,
    PairingClient pairingClient,
    PairingStore pairingRequests,
    IHubContext<UiHub, IUiClient> hub,
    IHubContext<RemoteHub, IRemoteClient> remote) : IHostedService
{
    public Task StartAsync(CancellationToken cancellationToken)
    {
        settings.Changed += PushSettingsAsync;
        secrets.ClientsChanged += PushClientsAsync;
        connection.Changed += PushConnectionAsync;
        usbServer.Changed += PushServerDevicesAsync;
        usbListener.StatusChanged += PushListenerAsync;
        usbClient.Changed += PushClientDevicesAsync;
        activity.Recorded += PushActivityAsync;
        finder.Changed += PushServersAsync;
        pairingClient.Changed += PushPairingStatusAsync;
        pairingRequests.Changed += PushPairingRequestsAsync;
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        settings.Changed -= PushSettingsAsync;
        secrets.ClientsChanged -= PushClientsAsync;
        connection.Changed -= PushConnectionAsync;
        usbServer.Changed -= PushServerDevicesAsync;
        usbListener.StatusChanged -= PushListenerAsync;
        usbClient.Changed -= PushClientDevicesAsync;
        activity.Recorded -= PushActivityAsync;
        finder.Changed -= PushServersAsync;
        pairingClient.Changed -= PushPairingStatusAsync;
        pairingRequests.Changed -= PushPairingRequestsAsync;
        return Task.CompletedTask;
    }

    private Task PushSettingsAsync(KoppelSettings value) => hub.Clients.All.SettingsChanged(value);

    private Task PushClientsAsync(IReadOnlyList<ClientView> value) => hub.Clients.All.ClientsChanged(value);

    private Task PushConnectionAsync(ConnectionView value) => hub.Clients.All.ConnectionChanged(value);

    private async Task PushServerDevicesAsync(ServerDevices value)
    {
        await hub.Clients.All.DevicesChanged(value);
        await PushRemoteDevicesAsync(value, settings.Current.ListenPort);
    }

    /// <summary>The listener bound a port. Clients learn it from the device list, which carries the USB port, so a port change reaches them here.</summary>
    private Task PushListenerAsync(UsbListenerStatus status) =>
        status.Listening ? PushRemoteDevicesAsync(usbServer.Current, status.Port) : Task.CompletedTask;

    /// <summary>Only clients that are still registered at their current generation get the list.</summary>
    private Task PushRemoteDevicesAsync(ServerDevices devices, int usbPort) =>
        remote.Clients.Clients(live.RemoteConnectionIds()).DevicesChanged(devices.ForRemote(usbPort));

    private Task PushClientDevicesAsync(ClientDevices value) => hub.Clients.All.ClientDevicesChanged(value);

    private Task PushActivityAsync(ActivityEvent value) => hub.Clients.All.ActivityRecorded(value);

    private Task PushServersAsync(IReadOnlyList<DiscoveredServer> value) => hub.Clients.All.ServersDiscovered(value);

    private Task PushPairingStatusAsync(PairingStatus value) => hub.Clients.All.PairingStatusChanged(value);

    private Task PushPairingRequestsAsync(PairingOverview value) => hub.Clients.All.PairingRequestsChanged(value);
}
