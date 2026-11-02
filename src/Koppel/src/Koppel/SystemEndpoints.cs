using System.Reflection;
using Koppel.Core.Usb;
using Koppel.Server;
using Koppel.Settings;
using Koppel.Usb;

namespace Koppel;

/// <summary>One thing Koppel needs from this machine, for the first-run checks.</summary>
internal sealed record SystemCheck(string Id, bool Ok, string Title, string Detail);

/// <summary>Facts about this process that never change while it runs.</summary>
internal sealed record HostInfo(DateTimeOffset StartedAt, int UiPort, int HttpsPort);

/// <summary>What the side panel shows about this machine.</summary>
internal sealed record SystemInfo(string Name, string Version, DateTimeOffset StartedAt, KoppelRole Role, int UiPort, int HttpsPort, string Fingerprint, string Os);

internal static class SystemEndpoints
{
    /// <summary>Koppel's version, such as 0.1.0, without the commit hash.</summary>
    public static string Version { get; } =
        typeof(SystemEndpoints).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion.Split('+')[0] ?? "dev";

    /// <summary>What this machine can do: share (usbip-host) and attach (vhci-hcd). Read-only.</summary>
    public static IEndpointRouteBuilder MapSystemEndpoints(this IEndpointRouteBuilder app)
    {
        app.MapGet("/api/system/checks", (IUsbHost host, IUsbAttacher attacher, SettingsStore settings, UsbServer server, UsbClient client) => TypedResults.Ok(new[]
        {
            host.Unavailable is { } shareProblem
                ? new SystemCheck("share", false, "Sharing is not available.", shareProblem)
                : new SystemCheck("share", true, "usbip-host is loaded.", "This machine can share devices."),
            attacher.Unavailable is { } attachProblem
                ? new SystemCheck("attach", false, "Attaching is not available.", attachProblem)
                : new SystemCheck("attach", true, "vhci-hcd is loaded.", "This machine can attach devices."),
        }.Concat(settings.Problem is { } problem
            ? [new SystemCheck("settings", false, "settings.json was not applied.", problem)]
            : [])
            .Concat(server.SharedProblem is { } sharedProblem
                ? [new SystemCheck("devices", false, "devices.json was not used.", sharedProblem)]
                : [])
            .Concat(client.RememberedProblem is { } rememberedProblem
                ? [new SystemCheck("attached", false, "attached.json was not used.", rememberedProblem)]
                : []).ToArray()));
        app.MapGet("/api/system/info", (HostInfo info, SettingsStore settings, ServerSecretsStore secrets) => TypedResults.Ok(new SystemInfo(
            Environment.MachineName,
            Version,
            info.StartedAt,
            settings.Current.Role,
            info.UiPort,
            info.HttpsPort,
            secrets.Fingerprint,
            OsLabel())));
        return app;
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
