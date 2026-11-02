using Koppel.Core.Usb;

namespace Koppel;

/// <summary>One thing Koppel needs from this machine, for the first-run checks.</summary>
internal sealed record SystemCheck(string Id, bool Ok, string Title, string Detail);

internal static class SystemEndpoints
{
    /// <summary>What this machine can do: share (usbip-host) and attach (vhci-hcd). Read-only.</summary>
    public static IEndpointRouteBuilder MapSystemEndpoints(this IEndpointRouteBuilder app)
    {
        app.MapGet("/api/system/checks", (IUsbHost host, IUsbAttacher attacher) => TypedResults.Ok(new[]
        {
            host.Unavailable is { } shareProblem
                ? new SystemCheck("share", false, "Sharing is not available.", shareProblem)
                : new SystemCheck("share", true, "usbip-host is loaded.", "This machine can share devices."),
            attacher.Unavailable is { } attachProblem
                ? new SystemCheck("attach", false, "Attaching is not available.", attachProblem)
                : new SystemCheck("attach", true, "vhci-hcd is loaded.", "This machine can attach devices."),
        }));
        return app;
    }
}
