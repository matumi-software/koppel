using Koppel.Core.Auth;
using Koppel.Server;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.SignalR;

namespace Koppel.Usb;

/// <summary>What a remote client sees of this server's devices: only the shared ones, without who holds them.</summary>
public sealed record RemoteDevices(IReadOnlyList<DeviceView> Devices);

/// <summary>
/// The server's certificate and, while a handover is prepared, the next one. It reaches the client over a
/// link pinned to a certificate the client already trusts, which is what lets the client trust the next one.
/// </summary>
/// <param name="Fingerprint">The certificate the server presents now.</param>
/// <param name="NextFingerprint">The certificate it will present after the switch, or null when none is prepared.</param>
public sealed record RemoteCertificates(string Fingerprint, string? NextFingerprint);

/// <summary>Messages the server pushes to remote Koppel clients.</summary>
/// <remarks>Public, with the types it carries, because SignalR emits its proxy in another assembly.</remarks>
public interface IRemoteClient
{
    Task DevicesChanged(RemoteDevices devices);

    /// <summary>Sent when a link opens and whenever the certificates change (prepare, cancel, switch).</summary>
    Task CertificatesChanged(RemoteCertificates certificates);
}

/// <summary>
/// Remote clients' live link, mapped at <c>/api/hubs/remote</c> on the network port. Needs a bearer token
/// from a registered client. The server pushes; the one call a client makes is to say it stored the next
/// certificate's fingerprint.
/// </summary>
[Authorize(Policy = KoppelAuthServer.ClientPolicy)]
internal sealed class RemoteHub(ServerSecretsStore secrets, UsbServer usb, LiveSessions live) : Hub<IRemoteClient>
{
    private const string SessionKey = "koppel.session";

    /// <summary>
    /// Refuses revoked clients and tokens from before a rotation, tracks the link so a later revoke or
    /// rotation ends it, and sends the new link the current list and certificates so it starts complete.
    /// </summary>
    public override async Task OnConnectedAsync()
    {
        if (!secrets.IsCurrent(Context.User))
        {
            Context.Abort();
            return;
        }

        Context.Items[SessionKey] = live.Add(Context.User!.GetClientId()!, Context.User!.GetGeneration()!.Value, Context.Abort, Context.ConnectionId);
        await Clients.Caller.DevicesChanged(usb.Current.ForRemote());
        await Clients.Caller.CertificatesChanged(new RemoteCertificates(secrets.Fingerprint, secrets.NextFingerprint));
        await base.OnConnectedAsync();
    }

    /// <summary>The client stored the next certificate's fingerprint. The server's UI shows who has it, so the admin knows when to switch.</summary>
    public async Task NextCertificateReceived(string fingerprint)
    {
        if (secrets.IsCurrent(Context.User) && !string.IsNullOrEmpty(fingerprint))
        {
            await secrets.MarkNextCertificateReceivedAsync(Context.User!.GetClientId()!, fingerprint, Context.ConnectionAborted);
        }
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
    public static RemoteDevices ForRemote(this ServerDevices devices) =>
        new([.. devices.Devices
            .Where(d => d.State is "shared" or "attached")
            .Select(d => new DeviceView(d.BusId, d.VidPid, d.Name, d.Manufacturer, d.Speed, d.State, null, null, null, null, null, d.Interfaces))]);
}
