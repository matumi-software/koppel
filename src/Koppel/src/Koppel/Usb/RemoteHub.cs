using Koppel.Core.Auth;
using Koppel.Server;
using Koppel.Settings;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.SignalR;

namespace Koppel.Usb;

/// <summary>What a remote client sees of this server's devices: only the shared ones, without who holds them.</summary>
public sealed record RemoteDevices(IReadOnlyList<DeviceView> Devices, int UsbPort);

/// <summary>Messages the server pushes to remote Koppel clients.</summary>
/// <remarks>Public, with the types it carries, because SignalR emits its proxy in another assembly.</remarks>
public interface IRemoteClient
{
    Task DevicesChanged(RemoteDevices devices);
}

/// <summary>
/// Remote clients' live link, mapped at <c>/api/hubs/remote</c> on the HTTPS port. Needs a bearer token
/// from a registered client. Push only.
/// </summary>
[Authorize(Policy = KoppelAuthServer.ClientPolicy)]
internal sealed class RemoteHub(ServerSecretsStore secrets, UsbServer usb, SettingsStore settings, LiveSessions live) : Hub<IRemoteClient>
{
    private const string SessionKey = "koppel.session";

    /// <summary>
    /// Refuses revoked clients and tokens from before a rotation, tracks the link so a later revoke or
    /// rotation ends it, and sends the new link the current list so it starts complete.
    /// </summary>
    public override async Task OnConnectedAsync()
    {
        if (!secrets.IsCurrent(Context.User))
        {
            Context.Abort();
            return;
        }

        Context.Items[SessionKey] = live.Add(Context.User!.GetClientId()!, Context.User!.GetGeneration()!.Value, Context.Abort, Context.ConnectionId);
        await Clients.Caller.DevicesChanged(usb.Current.ForRemote(settings.Current.ListenPort));
        await base.OnConnectedAsync();
    }

    public override Task OnDisconnectedAsync(Exception? exception)
    {
        (Context.Items.Remove(SessionKey, out var session) ? session as IDisposable : null)?.Dispose();
        return base.OnDisconnectedAsync(exception);
    }
}

internal static class RemoteDevicesExtensions
{
    /// <summary>
    /// What a remote client may see: the shared devices, with what identifies the device and what its UI shows.
    /// Who holds a device, the serial number, the driver and fault text stay on this machine. Another client
    /// learns only that the device is taken.
    /// </summary>
    public static RemoteDevices ForRemote(this ServerDevices devices, int usbPort) =>
        new([.. devices.Devices
            .Where(d => d.State is "shared" or "attached")
            .Select(d => new DeviceView(d.BusId, d.VidPid, d.Name, d.Manufacturer, d.Speed, d.State, null, null, null, null, null, d.Interfaces))], usbPort);
}
