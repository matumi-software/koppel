using System.ComponentModel;
using System.Diagnostics;
using System.Net;
using System.Runtime.Versioning;
using System.Security;
using System.ServiceProcess;
using Koppel.Web;
using Microsoft.Win32;
using static Koppel.Tray.TrayNative;

namespace Koppel.Tray;

/// <summary>
/// <c>KoppelDesktop.exe --open</c>, behind Koppel in the Start menu: opens the UI in the user's browser, and first starts
/// the service if it is not running. Starting a service takes an administrator, so that part runs again elevated as
/// <c>--start-service</c>, which asks through UAC. The installer ships this as <c>KoppelDesktop.exe</c>, a copy of
/// <c>Koppel.exe</c> marked as a Windows program, so the shortcut opens no console window.
/// </summary>
[SupportedOSPlatform("windows")]
internal static class KoppelOpener
{
    public const string Argument = "--open";
    public const string StartArgument = "--start-service";

    /// <summary>Where the service tells the desktop its UI address. Readable by every user; only the service writes it.</summary>
    private const string Key = @"SOFTWARE\Matumi\Koppel";
    private const string UiUrlValue = "UiUrl";
    private const string DefaultUiUrl = "https://localhost:5240/";

    private const int Cancelled = 1223; // ERROR_CANCELLED: the user said no to UAC
    private static readonly TimeSpan StartWait = TimeSpan.FromSeconds(30);
    private static readonly TimeSpan UiWait = TimeSpan.FromSeconds(60);

    private const string NotRunning = "Koppel is not running. Start it from Koppel in the Start menu, or start the Koppel "
        + "service in Services.";

    public static bool Requested(string[] args) => args.Length > 0 && args[0] is Argument or StartArgument;

    public static int Run(string[] args)
    {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        return args[0] == StartArgument ? StartService() : Open();
    }

    /// <summary>The service writes its UI address here when it starts, so the desktop opens the port it really uses.</summary>
    public static void PublishUiUrl(string uiUrl)
    {
        try
        {
            using var key = Registry.LocalMachine.CreateSubKey(Key);
            key.SetValue(UiUrlValue, uiUrl);
        }
        catch (Exception exception) when (exception is UnauthorizedAccessException or IOException or SecurityException)
        {
            // The Start menu entry then opens the default address, which is right unless the UI port was moved.
        }
    }

    private static string ReadUiUrl()
    {
        try
        {
            using var key = Registry.LocalMachine.OpenSubKey(Key);
            if (key?.GetValue(UiUrlValue) is string value && Uri.TryCreate(value, UriKind.Absolute, out var url) && url.Scheme == Uri.UriSchemeHttps)
            {
                return url.AbsoluteUri;
            }
        }
        catch (Exception exception) when (exception is UnauthorizedAccessException or IOException or SecurityException)
        {
        }

        return DefaultUiUrl;
    }

    private static int Open()
    {
        ServiceControllerStatus status;
        try
        {
            using var service = new ServiceController(KoppelService.Name);
            status = service.Status;
        }
        catch (InvalidOperationException)
        {
            Show("The Koppel service is not installed. Run Koppel-Setup.exe again to repair it.", MB_ICONERROR);
            return 1;
        }

        if (status is ServiceControllerStatus.Stopped or ServiceControllerStatus.StopPending)
        {
            if (Ask("Koppel is not running. Start it now?\n\nStarting it takes an administrator.") != IDYES)
            {
                return 1;
            }

            if (!StartElevated())
            {
                return 1;
            }
        }

        // The address is read after a start, because the service writes it as it starts.
        var uiUrl = ReadUiUrl();
        if (!WaitForUi(new Uri(uiUrl)))
        {
            Show($"Koppel is running, but its UI at {uiUrl} does not answer yet. Try again in a minute.", MB_ICONERROR);
            return 1;
        }

        try
        {
            Process.Start(new ProcessStartInfo(uiUrl) { UseShellExecute = true })?.Dispose();
            return 0;
        }
        catch (Win32Exception exception)
        {
            Show($"Koppel could not open {uiUrl}: {exception.Message}", MB_ICONERROR);
            return 1;
        }
    }

    /// <summary>Runs this program again as an administrator to start the service, and says why if that did not work.</summary>
    private static bool StartElevated()
    {
        var exe = Environment.ProcessPath;
        if (exe is null)
        {
            Show(NotRunning, MB_ICONERROR);
            return false;
        }

        try
        {
            using var elevated = Process.Start(new ProcessStartInfo(exe, StartArgument) { UseShellExecute = true, Verb = "runas" });
            if (elevated is null)
            {
                Show(NotRunning, MB_ICONERROR);
                return false;
            }

            elevated.WaitForExit();
            return elevated.ExitCode == 0; // it showed its own error otherwise
        }
        catch (Win32Exception exception) when (exception.NativeErrorCode == Cancelled)
        {
            Show(NotRunning, MB_ICONINFORMATION);
            return false;
        }
        catch (Win32Exception exception)
        {
            Show($"Koppel could not be started: {exception.Message}", MB_ICONERROR);
            return false;
        }
    }

    /// <summary><c>--start-service</c>, elevated: starts the service and waits until Windows says it runs.</summary>
    private static int StartService()
    {
        try
        {
            using var service = new ServiceController(KoppelService.Name);
            if (service.Status == ServiceControllerStatus.StopPending)
            {
                service.WaitForStatus(ServiceControllerStatus.Stopped, StartWait);
            }

            if (service.Status == ServiceControllerStatus.Stopped)
            {
                service.Start();
            }

            service.WaitForStatus(ServiceControllerStatus.Running, StartWait);
            return 0;
        }
        catch (Exception exception) when (exception is InvalidOperationException or Win32Exception or System.ServiceProcess.TimeoutException)
        {
            Show($"Koppel could not be started: {exception.InnerException?.Message ?? exception.Message}\n\nThe Application log in "
                + "Event Viewer (source Koppel) says more.", MB_ICONERROR);
            return 1;
        }
    }

    /// <summary>Asks the UI's health endpoint until it answers, as the Docker health check does.</summary>
    private static bool WaitForUi(Uri uiUrl)
    {
        var address = IPAddress.TryParse(uiUrl.Host, out var parsed) ? parsed : null;
        var deadline = DateTimeOffset.UtcNow + UiWait;
        while (true)
        {
            if (HealthProbe.RunAsync(address, uiUrl.Port).GetAwaiter().GetResult() == 0)
            {
                return true;
            }

            if (DateTimeOffset.UtcNow >= deadline)
            {
                return false;
            }

            Thread.Sleep(TimeSpan.FromSeconds(1));
        }
    }

    private static int Ask(string text) =>
        MessageBoxW(0, text, "Koppel", MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND);

    private static void Show(string text, uint icon) =>
        MessageBoxW(0, text, "Koppel", MB_OK | icon | MB_SETFOREGROUND);
}
