using Koppel.Settings;

namespace Koppel.Usb;

internal sealed record ShareRequest(bool Shared);

internal static class UsbEndpoints
{
    /// <summary>
    /// Server role. PUT writes the choice to <c>devices.json</c>, applies it to the kernel and answers
    /// 202; the outcome reaches the UI as <c>DevicesChanged</c>.
    /// </summary>
    public static IEndpointRouteBuilder MapUsbServerEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/server/devices");
        group.MapGet("/", (UsbServer usb) => TypedResults.Ok(usb.Current));
        group.MapPut("/{busId}/shared", async Task<IResult> (string busId, ShareRequest request, UsbServer usb, CancellationToken cancellationToken) =>
        {
            if (usb.Current.Unavailable is { } reason)
            {
                return TypedResults.ValidationProblem(new Dictionary<string, string[]> { ["device"] = [reason] });
            }

            await usb.SetSharedAsync(busId, request.Shared, cancellationToken);
            return TypedResults.Accepted((string?)null);
        });

        return app;
    }

    /// <summary>Remote clients: the shared devices. Added to the remote group, which checks the token and the client.</summary>
    public static RouteGroupBuilder MapUsbRemoteEndpoints(this RouteGroupBuilder remote)
    {
        remote.MapGet("/devices", (UsbServer usb, SettingsStore settings) =>
            TypedResults.Ok(usb.Current.ForRemote(settings.Current.ListenPort)));
        return remote;
    }

    /// <summary>
    /// Client role. Attach remembers the device, so it is attached again after a restart; detach forgets
    /// it. Both answer 202; the device shows as attached (or with its fault) when
    /// <c>ClientDevicesChanged</c> arrives.
    /// </summary>
    public static IEndpointRouteBuilder MapUsbClientEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/client/devices");
        group.MapGet("/", (UsbClient usb) => TypedResults.Ok(usb.Current));
        group.MapPost("/{busId}/attach", async (string busId, UsbClient usb) =>
        {
            await usb.BeginAttachAsync(busId);
            return TypedResults.Accepted((string?)null);
        });
        group.MapPost("/{busId}/detach", async (string busId, UsbClient usb) =>
        {
            await usb.DetachAsync(busId);
            return TypedResults.Accepted((string?)null);
        });
        return app;
    }
}
