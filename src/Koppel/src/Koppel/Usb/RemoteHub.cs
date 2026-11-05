using Skakel.Server;

namespace Koppel.Usb;

/// <summary>What a remote client sees of this server's devices: only the shared ones, without who holds them.</summary>
public sealed record RemoteDevices(IReadOnlyList<DeviceView> Devices);

/// <summary>Koppel's messages to remote Koppel clients. Skakel's own (the certificates, closing) come from <see cref="SkakelHub{TClient}"/>.</summary>
/// <remarks>Public, with the types it carries, because SignalR emits its proxy in another assembly.</remarks>
public interface IRemoteClient
{
    Task DevicesChanged(RemoteDevices devices);
}

/// <summary>
/// Remote clients' live link, mapped by Skakel at <c>/skakel/v1/hub</c> on the network port behind a
/// client's token. Skakel tells the client the server's certificates and ends the connection when the
/// client is revoked or rotated; Koppel pushes the shared devices.
/// </summary>
internal sealed class RemoteHub(UsbServer usb) : SkakelHub<IRemoteClient>
{
    /// <summary>The hub's path under <c>/skakel/v1/</c>, on both ends.</summary>
    public const string Path = "hub";

    /// <summary>Sends the new link the current list, so it starts complete.</summary>
    public override async Task OnConnectedAsync()
    {
        await base.OnConnectedAsync();
        if (Context.ConnectionAborted.IsCancellationRequested)
        {
            return;
        }

        await Clients.Caller.DevicesChanged(usb.Current.ForRemote());
    }
}

internal static class RemoteDevicesExtensions
{
    /// <summary>
    /// What a remote client may see: the shared devices, with what identifies the device and what its UI shows.
    /// Who holds a device, the serial number, the driver and fault text stay on this machine. Another client
    /// learns only that the device is taken.
    /// </summary>
    public static RemoteDevices ForRemote(this ServerDevices devices) =>
        new([.. devices.Devices
            .Where(d => d.State is "shared" or "attached")
            .Select(d => new DeviceView(d.BusId, d.VidPid, d.Name, d.Manufacturer, d.Speed, d.State, null, null, null, null, null, d.Interfaces))]);
}
