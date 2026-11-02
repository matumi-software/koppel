using System.Net.WebSockets;
using Koppel.Core.Usb;

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
    /// Remote clients: the shared devices, and the USB data plane as a WebSocket on the HTTPS port. Added
    /// to the remote group, which checks the token and the client.
    /// </summary>
    public static RouteGroupBuilder MapUsbRemoteEndpoints(this RouteGroupBuilder remote)
    {
        remote.MapGet("/devices", (UsbServer usb) => TypedResults.Ok(usb.Current.ForRemote()));

        // Map rather than MapGet: a WebSocket over HTTP/2 arrives as CONNECT.
        remote.Map("/usb", async Task<IResult> (HttpContext http, UsbDataPlane plane) =>
        {
            if (!http.WebSockets.IsWebSocketRequest)
            {
                return TypedResults.BadRequest();
            }

            var socket = await http.WebSockets.AcceptWebSocketAsync(new WebSocketAcceptContext
            {
                KeepAliveInterval = UsbDataPlane.KeepAliveInterval,
                KeepAliveTimeout = UsbDataPlane.KeepAliveTimeout,
            });
            await using var session = WebSocketStream.Create(socket, WebSocketMessageType.Binary, closeTimeout: TimeSpan.FromSeconds(2));
            await plane.ServeAsync(session, http.User, http.RequestAborted);
            return TypedResults.Empty;
        });
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
