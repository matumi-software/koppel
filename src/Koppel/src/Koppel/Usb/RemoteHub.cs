using Koppel.Core.Auth;
using Koppel.Server;
using Koppel.Settings;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.SignalR;

namespace Koppel.Usb;

/// <summary>What a remote client sees of this server's devices: only the shared ones.</summary>
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
internal sealed class RemoteHub(ServerSecretsStore secrets, UsbServer usb, SettingsStore settings) : Hub<IRemoteClient>
{
    /// <summary>Refuses revoked clients, and sends a new link the current list so it starts complete.</summary>
    public override async Task OnConnectedAsync()
    {
        if (!secrets.IsRegistered(Context.User?.GetClientId()))
        {
            Context.Abort();
            return;
        }

        await Clients.Caller.DevicesChanged(usb.Current.ForRemote(settings.Current.ListenPort));
        await base.OnConnectedAsync();
    }
}

internal static class RemoteDevicesExtensions
{
    public static RemoteDevices ForRemote(this ServerDevices devices, int usbPort) =>
        new([.. devices.Devices.Where(d => d.State is "shared" or "attached")], usbPort);
}
