using Koppel.Client;
using Koppel.Core.Activity;
using Koppel.Discovery;
using Koppel.Login;
using Koppel.Settings;
using Koppel.Usb;
using Microsoft.AspNetCore.SignalR;
using Skakel.Server;

namespace Koppel.Hubs;

/// <summary>Messages the server pushes to the UI. Each carries the value as written to disk.</summary>
/// <remarks>Public, with the types it carries, because SignalR emits its proxy in another assembly.</remarks>
public interface IUiClient
{
    /// <summary>The UI treats a settings change as done only when this arrives.</summary>
    Task SettingsChanged(KoppelSettings settings);

    /// <summary>Server role: the registered clients after a create, a pairing, a revoke or a rotation.</summary>
    Task ClientsChanged(IReadOnlyList<SkakelClientView> clients);

    /// <summary>Server role: the certificate and token signing key after a rotation, a prepare, a cancel or a switch.</summary>
    Task ServerKeysChanged(SkakelKeysView keys);

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
    Task PairingRequestsChanged(SkakelPairingOverview pairing);
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
    SkakelClients clients,
    SkakelKeys keys,
    ConnectionStore connection,
    UsbServer usbServer,
    UsbClient usbClient,
    InMemoryActivityStore activity,
    ServerFinder finder,
    PairingClient pairingClient,
    SkakelPairing pairingRequests,
    IHubContext<UiHub, IUiClient> hub,
    IHubContext<RemoteHub, IRemoteClient> remote) : IHostedService
{
    public Task StartAsync(CancellationToken cancellationToken)
    {
        settings.Changed += PushSettingsAsync;
        clients.Changed += PushClientsAsync;
        keys.Changed += PushKeysAsync;
        connection.Changed += PushConnectionAsync;
        usbServer.Changed += PushServerDevicesAsync;
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
        clients.Changed -= PushClientsAsync;
        keys.Changed -= PushKeysAsync;
        connection.Changed -= PushConnectionAsync;
        usbServer.Changed -= PushServerDevicesAsync;
        usbClient.Changed -= PushClientDevicesAsync;
        activity.Recorded -= PushActivityAsync;
        finder.Changed -= PushServersAsync;
        pairingClient.Changed -= PushPairingStatusAsync;
        pairingRequests.Changed -= PushPairingRequestsAsync;
        return Task.CompletedTask;
    }

    private Task PushSettingsAsync(KoppelSettings value) => hub.Clients.All.SettingsChanged(value);

    private Task PushClientsAsync(IReadOnlyList<SkakelClientView> value) => hub.Clients.All.ClientsChanged(value);

    /// <summary>The UI gets the whole state. Skakel tells the paired clients of the certificates itself.</summary>
    private Task PushKeysAsync(SkakelKeysView value) => hub.Clients.All.ServerKeysChanged(value);

    private Task PushConnectionAsync(ConnectionView value) => hub.Clients.All.ConnectionChanged(value);

    private async Task PushServerDevicesAsync(ServerDevices value)
    {
        await hub.Clients.All.DevicesChanged(value);
        await PushRemoteDevicesAsync(value);
    }

    /// <summary>Every linked client gets the list. Skakel ends a client's connections as soon as it is revoked or rotated.</summary>
    private Task PushRemoteDevicesAsync(ServerDevices devices) =>
        remote.Clients.All.DevicesChanged(devices.ForRemote());

    private Task PushClientDevicesAsync(ClientDevices value) => hub.Clients.All.ClientDevicesChanged(value);

    private Task PushActivityAsync(ActivityEvent value) => hub.Clients.All.ActivityRecorded(value);

    private Task PushServersAsync(IReadOnlyList<DiscoveredServer> value) => hub.Clients.All.ServersDiscovered(value);

    private Task PushPairingStatusAsync(PairingStatus value) => hub.Clients.All.PairingStatusChanged(value);

    private Task PushPairingRequestsAsync(SkakelPairingOverview value) => hub.Clients.All.PairingRequestsChanged(value);
}
