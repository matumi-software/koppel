using System.Reflection;
using Koppel.Core.Usb;
using Koppel.Settings;
using Koppel.Usb;

namespace Koppel;

/// <summary>One thing Koppel needs from this machine, for the first-run checks.</summary>
internal sealed record SystemCheck(string Id, bool Ok, string Title, string Detail);

/// <summary>Facts about this process that never change while it runs. The UI and network ports are the same port unless <c>Ports:Network</c> is set.</summary>
internal sealed record HostInfo(DateTimeOffset StartedAt, int UiPort, int NetworkPort, int DiscoveryPort);

/// <summary>What the side panel shows about this machine.</summary>
internal sealed record SystemInfo(string Name, string Version, DateTimeOffset StartedAt, KoppelRole Role, int UiPort, int HttpsPort, string Fingerprint, string Os);

internal static class SystemEndpoints
{
    /// <summary>Koppel's app id on Skakel: discovery answers and the info endpoint carry it, and a client lists only servers with it.</summary>
    public const string AppId = "koppel";

    /// <summary>Koppel's version, such as 0.1.0, without the commit hash.</summary>
    public static string Version { get; } =
        typeof(SystemEndpoints).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion.Split('+')[0] ?? "dev";

    /// <summary>What this machine can do: share (usbip-host) and attach (vhci-hcd). Read-only.</summary>
    public static IEndpointRouteBuilder MapSystemEndpoints(this IEndpointRouteBuilder app)
    {
        app.MapGet("/api/system/checks", (IUsbHost host, IUsbAttacher attacher, IUsbIpDriver driver, SettingsStore settings, UsbServer server, UsbClient client) => TypedResults.Ok(new[]
        {
            ShareCheck(host, OperatingSystem.IsWindows()),
            AttachCheck(attacher, driver, OperatingSystem.IsWindows()),
        }.Concat(settings.Problem is { } problem
            ? [new SystemCheck("settings", false, "settings.json was not applied.", problem)]
            : [])
            .Concat(server.SharedProblem is { } sharedProblem
                ? [new SystemCheck("devices", false, "devices.json was not used.", sharedProblem)]
                : [])
            .Concat(client.RememberedProblem is { } rememberedProblem
                ? [new SystemCheck("attached", false, "attached.json was not used.", rememberedProblem)]
                : []).ToArray()));
        app.MapGet("/api/system/info", (HostInfo info, SettingsStore settings, Skakel.Server.SkakelKeys keys) => TypedResults.Ok(new SystemInfo(
            Environment.MachineName,
            Version,
            info.StartedAt,
            settings.Current.Role,
            info.UiPort,
            info.NetworkPort,
            keys.Fingerprint,
            OsLabel())));
        return app;
    }

    /// <summary>
    /// On Linux the host is usbip-host. On native Windows Koppel cannot share by itself: a server there is usbipd-win
    /// reached through <c>Usb:Upstream</c>, which <see cref="UsbIpUpstreamHost"/> reports on when it is set.
    /// </summary>
    internal static SystemCheck ShareCheck(IUsbHost host, bool nativeWindows) => host switch
    {
        UnsupportedUsb when nativeWindows => new SystemCheck(
            "share",
            false,
            "Sharing needs usbipd-win.",
            "A Windows server shares through usbipd-win. Install it and set Usb:Upstream to its address. See docs/windows.md."),
        { Unavailable: { } problem } => new SystemCheck("share", false, "Sharing is not available.", problem),
        UsbIpUpstreamHost when nativeWindows => new SystemCheck("share", true, "usbipd-win is reachable.", "This machine shares the devices of the USB/IP server in Usb:Upstream."),
        _ => new SystemCheck("share", true, "usbip-host is loaded.", "This machine can share devices."),
    };

    /// <summary>On Linux the kernel's vhci-hcd attaches. On native Windows it is usbip-win2's driver, which <see cref="IUsbIpDriver"/> already checks for.</summary>
    internal static SystemCheck AttachCheck(IUsbAttacher attacher, IUsbIpDriver driver, bool nativeWindows)
    {
        if (nativeWindows)
        {
            return driver.Unavailable is { } driverProblem
                ? new SystemCheck("attach", false, "The usbip-win2 driver is not installed.", driverProblem)
                : new SystemCheck("attach", true, "The usbip-win2 driver is installed.", "This machine can attach devices.");
        }

        return attacher.Unavailable is { } attachProblem
            ? new SystemCheck("attach", false, "Attaching is not available.", attachProblem)
            : new SystemCheck("attach", true, "vhci-hcd is loaded.", "This machine can attach devices.");
    }

    /// <summary>On Linux the kernel matters most (it carries the USB/IP drivers), and in Docker it is the host's.</summary>
    private static string OsLabel()
    {
        if (OperatingSystem.IsLinux() && File.Exists("/proc/sys/kernel/osrelease"))
        {
            var kernel = File.ReadAllText("/proc/sys/kernel/osrelease").Trim();
            var inContainer = Environment.GetEnvironmentVariable("DOTNET_RUNNING_IN_CONTAINER") == "true";
            return inContainer ? $"Linux kernel {kernel}, in Docker" : $"Linux kernel {kernel}";
        }

        return System.Runtime.InteropServices.RuntimeInformation.OSDescription;
    }
}
