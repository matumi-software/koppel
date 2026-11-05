using Koppel.Core.Usb;
using Skakel.Server;

namespace Koppel.Usb;

internal sealed record ShareRequest(bool Shared);

internal static class UsbEndpoints
{
    /// <summary>
    /// Server role. PUT applies the choice to the kernel, saves it in <c>devices.json</c> once it took effect and
    /// answers 202; the outcome reaches the UI as <c>DevicesChanged</c>. A bus ID that is not one answers 400.
    /// </summary>
    public static IEndpointRouteBuilder MapUsbServerEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/api/server/devices");
        group.MapGet("/", (UsbServer usb) => TypedResults.Ok(usb.Current));
        group.MapPut("/{busId}/shared", async Task<IResult> (string busId, ShareRequest request, UsbServer usb, CancellationToken cancellationToken) =>
        {
            if (!UsbBusId.IsValid(busId))
            {
                return TypedResults.ValidationProblem(new Dictionary<string, string[]> { ["busId"] = ["That is not a USB bus ID, such as 1-1.4."] });
            }

            if (usb.Current.Unavailable is { } reason)
            {
                return TypedResults.ValidationProblem(new Dictionary<string, string[]> { ["device"] = [reason] });
            }

            await usb.SetSharedAsync(busId, request.Shared, cancellationToken);
            return TypedResults.Accepted((string?)null);
        });

        return app;
    }

    /// <summary>
    /// Remote clients: the USB data plane, Koppel's <c>usb</c> stream type at <c>/skakel/v1/streams/usb</c>.
    /// Skakel checks the token and the client before <see cref="UsbDataPlane.OpenAsync"/> runs.
    /// </summary>
    public static IEndpointRouteBuilder MapUsbStream(this IEndpointRouteBuilder app)
    {
        var plane = app.ServiceProvider.GetRequiredService<UsbDataPlane>();
        app.MapSkakelStream(UsbDataPlane.StreamType, plane.OpenAsync);
        return app;
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
        group.MapPost("/{busId}/attach", async Task<IResult> (string busId, UsbClient usb) =>
        {
            if (!UsbBusId.IsValid(busId))
            {
                return TypedResults.ValidationProblem(new Dictionary<string, string[]> { ["busId"] = ["That is not a USB bus ID."] });
            }

            await usb.BeginAttachAsync(busId);
            return TypedResults.Accepted((string?)null);
        });
        group.MapPost("/{busId}/detach", async Task<IResult> (string busId, UsbClient usb) =>
        {
            if (!UsbBusId.IsValid(busId))
            {
                return TypedResults.ValidationProblem(new Dictionary<string, string[]> { ["busId"] = ["That is not a USB bus ID."] });
            }

            await usb.DetachAsync(busId);
            return TypedResults.Accepted((string?)null);
        });
        return app;
    }
}
