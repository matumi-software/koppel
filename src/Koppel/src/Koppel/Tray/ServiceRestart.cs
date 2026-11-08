using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.Versioning;
using System.ServiceProcess;

namespace Koppel.Tray;

/// <summary>
/// Restarts the Windows service for the tray icon's <b>Restart Koppel</b>. A service cannot start itself again once it has
/// stopped, so before it stops it starts <c>Koppel.exe --restart-service</c>, which runs on as LocalSystem, waits for the
/// service to stop and starts it. The service stops cleanly, so this is not a failure and the recovery actions stay out of it.
/// </summary>
[SupportedOSPlatform("windows")]
internal static class ServiceRestart
{
    public const string Argument = "--restart-service";

    private static readonly TimeSpan StopWait = TimeSpan.FromMinutes(2);
    private static readonly TimeSpan StartWait = TimeSpan.FromMinutes(1);

    public static bool Requested(string[] args) => args.Length > 0 && args[0] == Argument;

    /// <summary>Called by the service: starts the helper that brings it back. False when it could not be started.</summary>
    public static bool Begin()
    {
        var exe = Environment.ProcessPath;
        if (exe is null)
        {
            return false;
        }

        try
        {
            using var helper = Process.Start(new ProcessStartInfo(exe, Argument)
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WorkingDirectory = Path.GetDirectoryName(exe),
            });
            return helper is not null;
        }
        catch (Win32Exception)
        {
            return false;
        }
    }

    /// <summary><c>--restart-service</c>: waits for the service to stop, then starts it. 0 when it runs again.</summary>
    public static int Run()
    {
        try
        {
            using var service = new ServiceController(KoppelService.Name);
            service.WaitForStatus(ServiceControllerStatus.Stopped, StopWait);
            service.Start();
            service.WaitForStatus(ServiceControllerStatus.Running, StartWait);
            return 0;
        }
        catch (Exception exception) when (exception is InvalidOperationException or Win32Exception or System.ServiceProcess.TimeoutException)
        {
            // Nobody sees this process. The service logged that it was restarting; if it does not come back, it starts with
            // Windows, and from Koppel in the Start menu.
            return 1;
        }
    }
}
