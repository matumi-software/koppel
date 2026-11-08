using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using System.ServiceProcess;
using static Koppel.Tray.TrayNative;

namespace Koppel.Tray;

/// <summary>
/// <c>Koppel.exe --tray &lt;ui url&gt;</c>: the icon in the notification area of a signed-in user's session. The service
/// starts it there (<see cref="TrayLauncher"/>) with the user's own token, the way Apollo runs its tray. Clicking it opens
/// the UI in the user's browser; its menu also shuts the service down. Its standard input is a pipe from the service: when
/// that closes, because the service stopped or died, the icon is removed and the process ends.
/// </summary>
[SupportedOSPlatform("windows")]
internal static unsafe class TrayIcon
{
    public const string Argument = "--tray";

    private const uint CallbackMessage = WM_APP + 1;
    private const uint IconId = 1;
    private const int OpenCommand = 1;
    private const int ShutDownCommand = 2;
    private const int RestartCommand = 3;

    private static nint s_window;
    private static nint s_icon;
    private static nint s_menu;
    private static uint s_taskbarCreated;
    private static string s_uiUrl = "";

    public static bool Requested(string[] args) => args.Length > 0 && args[0] == Argument;

    public static int Run(string[] args)
    {
        if (args.Length < 2 || !Uri.TryCreate(args[1], UriKind.Absolute, out var url) || url.Scheme != Uri.UriSchemeHttps)
        {
            return 2;
        }

        s_uiUrl = url.AbsoluteUri;
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        var instance = GetModuleHandleW(0);
        fixed (char* className = "KoppelTray")
        {
            var windowClass = new WNDCLASSEXW
            {
                cbSize = (uint)sizeof(WNDCLASSEXW),
                lpfnWndProc = &WindowProc,
                hInstance = instance,
                lpszClassName = className,
            };
            if (RegisterClassExW(&windowClass) == 0)
            {
                return 1;
            }
        }

        // A hidden top-level window, not a message-only one: only top-level windows hear TaskbarCreated.
        s_window = CreateWindowExW(0, "KoppelTray", "Koppel", 0, 0, 0, 0, 0, 0, 0, instance, 0);
        if (s_window == 0)
        {
            return 1;
        }

        s_taskbarCreated = RegisterWindowMessageW("TaskbarCreated");
        s_icon = LoadImageW(instance, IconResource, IMAGE_ICON, GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), 0);
        s_menu = CreatePopupMenu();
        AppendMenuW(s_menu, MF_STRING, OpenCommand, "Open Koppel");
        AppendMenuW(s_menu, MF_SEPARATOR, 0, null);
        AppendMenuW(s_menu, MF_STRING, RestartCommand, "Restart Koppel");
        AppendMenuW(s_menu, MF_STRING, ShutDownCommand, "Shut down Koppel");
        SetMenuDefaultItem(s_menu, OpenCommand, 0);
        AddIcon();

        var window = s_window;
        new Thread(() => WaitForService(window)) { IsBackground = true, Name = "Koppel service pipe" }.Start();

        MSG message;
        while (GetMessageW(&message, 0, 0, 0) > 0)
        {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        return 0;
    }

    /// <summary>Blocks until the service closes its end of the pipe on standard input, then closes the window.</summary>
    private static void WaitForService(nint window)
    {
        try
        {
            using var pipe = Console.OpenStandardInput();
            var buffer = new byte[64];
            while (pipe.Read(buffer, 0, buffer.Length) > 0)
            {
            }
        }
        catch (IOException)
        {
        }

        PostMessageW(window, WM_CLOSE, 0, 0);
    }

    [UnmanagedCallersOnly]
    private static nint WindowProc(nint hwnd, uint message, nint wParam, nint lParam)
    {
        try
        {
            if (message == CallbackMessage)
            {
                // NOTIFYICON_VERSION_4: the event is in the low word of lParam, the anchor point in wParam.
                switch ((uint)(lParam & 0xFFFF))
                {
                    case NIN_SELECT:
                    case NIN_KEYSELECT:
                        OpenUi();
                        break;
                    case WM_CONTEXTMENU:
                        ShowMenu(hwnd, (short)(wParam & 0xFFFF), (short)((wParam >> 16) & 0xFFFF));
                        break;
                }

                return 0;
            }

            if (message == s_taskbarCreated && s_taskbarCreated != 0)
            {
                AddIcon(); // Explorer restarted and lost every icon
                return 0;
            }

            switch (message)
            {
                case WM_CLOSE:
                    RemoveIcon();
                    DestroyWindow(hwnd);
                    return 0;
                case WM_DESTROY:
                    PostQuitMessage(0);
                    return 0;
            }
        }
        catch (Exception exception) when (exception is not OutOfMemoryException)
        {
            // An exception must not cross into user32. The icon stays; the user can try again.
        }

        return DefWindowProcW(hwnd, message, wParam, lParam);
    }

    private static void AddIcon()
    {
        var data = IconData();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = CallbackMessage;
        data.hIcon = s_icon;
        "Koppel".AsSpan().CopyTo(new Span<char>(data.szTip, 127));
        Shell_NotifyIconW(NIM_ADD, &data);
        data.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &data);
    }

    private static void RemoveIcon()
    {
        var data = IconData();
        Shell_NotifyIconW(NIM_DELETE, &data);
    }

    private static NOTIFYICONDATAW IconData() => new()
    {
        cbSize = (uint)sizeof(NOTIFYICONDATAW),
        hWnd = s_window,
        uID = IconId,
    };

    private static void ShowMenu(nint hwnd, int x, int y)
    {
        // Without the foreground the menu does not close when the user clicks elsewhere; WM_NULL after it is the documented fix.
        SetForegroundWindow(hwnd);
        var command = TrackPopupMenuEx(s_menu, TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY, x, y, hwnd, 0);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        switch (command)
        {
            case OpenCommand:
                OpenUi();
                break;
            case ShutDownCommand:
                ShutDown(hwnd);
                break;
            case RestartCommand:
                Restart(hwnd);
                break;
        }
    }

    private static void OpenUi()
    {
        try
        {
            Process.Start(new ProcessStartInfo(s_uiUrl) { UseShellExecute = true })?.Dispose();
        }
        catch (Win32Exception exception)
        {
            MessageBoxW(s_window, $"Koppel could not open {s_uiUrl}: {exception.Message}", "Koppel", MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
        }
    }

    private static void ShutDown(nint hwnd)
    {
        const string question = "Shut down Koppel?\n\nDevices this PC shares or has attached stop working until Koppel starts again. "
            + "It starts again with Windows, or from Koppel in the Start menu.";
        AskAndSend(hwnd, question, KoppelService.ShutDownCommand, "shut down");
    }

    private static void Restart(nint hwnd)
    {
        const string question = "Restart Koppel?\n\nDevices this PC shares or has attached pause while it restarts, and clients "
            + "attach theirs again when it is back. If no password is set yet, setup opens again.";
        AskAndSend(hwnd, question, KoppelService.RestartCommand, "restarted");
    }

    private static void AskAndSend(nint hwnd, string question, int command, string failed)
    {
        if (MessageBoxW(hwnd, question, "Koppel", MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND) != IDYES)
        {
            return;
        }

        // Interactive users may send a service its user-defined controls but may not stop or start it, so the service does it
        // itself (KoppelServiceLifetime). When it stops, the pipe closes and this icon goes with it; after a restart the
        // service starts a new one.
        try
        {
            using var service = new ServiceController(KoppelService.Name);
            service.ExecuteCommand(command);
        }
        catch (Exception exception) when (exception is InvalidOperationException or Win32Exception)
        {
            MessageBoxW(hwnd, $"Koppel could not be {failed}: {exception.InnerException?.Message ?? exception.Message}", "Koppel",
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
        }
    }
}
