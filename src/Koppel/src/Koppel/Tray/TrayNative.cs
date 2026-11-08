using System.Runtime.InteropServices;
using System.Runtime.Versioning;

namespace Koppel.Tray;

/// <summary>The Win32 calls behind the tray icon and the service that launches it.</summary>
[SupportedOSPlatform("windows")]
internal static unsafe partial class TrayNative
{
    // Window messages and styles.
    public const uint WM_NULL = 0x0000;
    public const uint WM_DESTROY = 0x0002;
    public const uint WM_CLOSE = 0x0010;
    public const uint WM_CONTEXTMENU = 0x007B;
    public const uint WM_APP = 0x8000;

    // Shell_NotifyIcon.
    public const uint NIM_ADD = 0;
    public const uint NIM_MODIFY = 1;
    public const uint NIM_DELETE = 2;
    public const uint NIM_SETVERSION = 4;
    public const uint NIF_MESSAGE = 0x01;
    public const uint NIF_ICON = 0x02;
    public const uint NIF_TIP = 0x04;
    public const uint NIF_SHOWTIP = 0x80;
    public const uint NOTIFYICON_VERSION_4 = 4;
    public const uint NIN_SELECT = 0x0400;
    public const uint NIN_KEYSELECT = 0x0401;

    // Menus and message boxes.
    public const uint MF_STRING = 0x0000;
    public const uint MF_SEPARATOR = 0x0800;
    public const uint TPM_RIGHTBUTTON = 0x0002;
    public const uint TPM_NONOTIFY = 0x0080;
    public const uint TPM_RETURNCMD = 0x0100;
    public const uint MB_OK = 0x0000;
    public const uint MB_YESNO = 0x0004;
    public const uint MB_ICONERROR = 0x0010;
    public const uint MB_ICONQUESTION = 0x0020;
    public const uint MB_ICONINFORMATION = 0x0040;
    public const uint MB_DEFBUTTON2 = 0x0100;
    public const uint MB_SETFOREGROUND = 0x00010000;
    public const int IDYES = 6;

    // Icons and DPI.
    public const uint IMAGE_ICON = 1;
    public const int SM_CXSMICON = 49;
    public const int SM_CYSMICON = 50;
    public const int IconResource = 32512; // the .NET SDK embeds ApplicationIcon under this id
    public static readonly nint DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4;

    // Sessions and processes.
    public const int WTSActive = 0;
    public const uint CREATE_UNICODE_ENVIRONMENT = 0x00000400;
    public const uint EXTENDED_STARTUPINFO_PRESENT = 0x00080000;
    public const uint CREATE_NO_WINDOW = 0x08000000;
    public const uint STARTF_USESTDHANDLES = 0x00000100;
    public static readonly nint PROC_THREAD_ATTRIBUTE_HANDLE_LIST = 0x00020002;
    public const uint WAIT_TIMEOUT = 0x00000102;

    [StructLayout(LayoutKind.Sequential)]
    public struct WNDCLASSEXW
    {
        public uint cbSize;
        public uint style;
        public delegate* unmanaged<nint, uint, nint, nint, nint> lpfnWndProc;
        public int cbClsExtra;
        public int cbWndExtra;
        public nint hInstance;
        public nint hIcon;
        public nint hCursor;
        public nint hbrBackground;
        public char* lpszMenuName;
        public char* lpszClassName;
        public nint hIconSm;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct MSG
    {
        public nint hwnd;
        public uint message;
        public nint wParam;
        public nint lParam;
        public uint time;
        public int ptX;
        public int ptY;
        public uint lPrivate;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct NOTIFYICONDATAW
    {
        public uint cbSize;
        public nint hWnd;
        public uint uID;
        public uint uFlags;
        public uint uCallbackMessage;
        public nint hIcon;
        public fixed char szTip[128];
        public uint dwState;
        public uint dwStateMask;
        public fixed char szInfo[256];
        public uint uVersion;
        public fixed char szInfoTitle[64];
        public uint dwInfoFlags;
        public Guid guidItem;
        public nint hBalloonIcon;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct WTS_SESSION_INFOW
    {
        public uint SessionId;
        public char* pWinStationName;
        public int State;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct STARTUPINFOW
    {
        public uint cb;
        public char* lpReserved;
        public char* lpDesktop;
        public char* lpTitle;
        public uint dwX;
        public uint dwY;
        public uint dwXSize;
        public uint dwYSize;
        public uint dwXCountChars;
        public uint dwYCountChars;
        public uint dwFillAttribute;
        public uint dwFlags;
        public ushort wShowWindow;
        public ushort cbReserved2;
        public nint lpReserved2;
        public nint hStdInput;
        public nint hStdOutput;
        public nint hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct STARTUPINFOEXW
    {
        public STARTUPINFOW StartupInfo;
        public nint lpAttributeList;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION
    {
        public nint hProcess;
        public nint hThread;
        public uint dwProcessId;
        public uint dwThreadId;
    }

    // user32: the window, the icon's menu and the message loop.
    [LibraryImport("user32.dll", SetLastError = true)]
    public static partial ushort RegisterClassExW(WNDCLASSEXW* wndClass);

    [LibraryImport("user32.dll", SetLastError = true, StringMarshalling = StringMarshalling.Utf16)]
    public static partial nint CreateWindowExW(uint exStyle, string className, string windowName, uint style, int x, int y, int width, int height, nint parent, nint menu, nint instance, nint param);

    [LibraryImport("user32.dll")]
    public static partial nint DefWindowProcW(nint hwnd, uint message, nint wParam, nint lParam);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool DestroyWindow(nint hwnd);

    [LibraryImport("user32.dll")]
    public static partial void PostQuitMessage(int exitCode);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool PostMessageW(nint hwnd, uint message, nint wParam, nint lParam);

    [LibraryImport("user32.dll")]
    public static partial int GetMessageW(MSG* message, nint hwnd, uint filterMin, uint filterMax);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool TranslateMessage(MSG* message);

    [LibraryImport("user32.dll")]
    public static partial nint DispatchMessageW(MSG* message);

    [LibraryImport("user32.dll", StringMarshalling = StringMarshalling.Utf16)]
    public static partial uint RegisterWindowMessageW(string name);

    [LibraryImport("user32.dll")]
    public static partial nint CreatePopupMenu();

    [LibraryImport("user32.dll", StringMarshalling = StringMarshalling.Utf16)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool AppendMenuW(nint menu, uint flags, nuint id, string? text);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool SetMenuDefaultItem(nint menu, uint item, uint byPosition);

    [LibraryImport("user32.dll")]
    public static partial int TrackPopupMenuEx(nint menu, uint flags, int x, int y, nint hwnd, nint parameters);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool SetForegroundWindow(nint hwnd);

    [LibraryImport("user32.dll", StringMarshalling = StringMarshalling.Utf16)]
    public static partial int MessageBoxW(nint hwnd, string text, string caption, uint type);

    [LibraryImport("user32.dll")]
    public static partial nint LoadImageW(nint instance, nint name, uint type, int width, int height, uint load);

    [LibraryImport("user32.dll")]
    public static partial int GetSystemMetrics(int index);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool SetProcessDpiAwarenessContext(nint context);

    // shell32 and kernel32 for the tray process.
    [LibraryImport("shell32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool Shell_NotifyIconW(uint message, NOTIFYICONDATAW* data);

    [LibraryImport("kernel32.dll")]
    public static partial nint GetModuleHandleW(nint moduleName);

    // wtsapi32 and userenv: the signed-in sessions and their users.
    [LibraryImport("wtsapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool WTSEnumerateSessionsW(nint server, uint reserved, uint version, out WTS_SESSION_INFOW* sessions, out uint count);

    [LibraryImport("wtsapi32.dll")]
    public static partial void WTSFreeMemory(void* memory);

    [LibraryImport("wtsapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool WTSQueryUserToken(uint sessionId, out nint token);

    [LibraryImport("userenv.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool CreateEnvironmentBlock(out nint environment, nint token, [MarshalAs(UnmanagedType.Bool)] bool inherit);

    [LibraryImport("userenv.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool DestroyEnvironmentBlock(nint environment);

    // advapi32 and kernel32: starting the tray in a session.
    [LibraryImport("advapi32.dll", SetLastError = true, StringMarshalling = StringMarshalling.Utf16)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool CreateProcessAsUserW(nint token, string? applicationName, char* commandLine, nint processAttributes, nint threadAttributes,
        [MarshalAs(UnmanagedType.Bool)] bool inheritHandles, uint creationFlags, nint environment, string? currentDirectory, STARTUPINFOEXW* startupInfo,
        out PROCESS_INFORMATION processInformation);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool InitializeProcThreadAttributeList(nint attributeList, int attributeCount, uint flags, ref nint size);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool UpdateProcThreadAttribute(nint attributeList, uint flags, nint attribute, nint* value, nint size, nint previousValue, nint returnSize);

    [LibraryImport("kernel32.dll")]
    public static partial void DeleteProcThreadAttributeList(nint attributeList);

    [LibraryImport("kernel32.dll")]
    public static partial uint WaitForSingleObject(SafeHandle handle, uint milliseconds);

    [LibraryImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool TerminateProcess(SafeHandle process, uint exitCode);

    [LibraryImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static partial bool CloseHandle(nint handle);
}
