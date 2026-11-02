using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Koppel.Core.Usb.Linux;

/// <summary>The kernel's usbfs device nodes (<c>/dev/bus/usb/BBB/DDD</c>), for what sysfs cannot do.</summary>
internal static partial class UsbDeviceFs
{
    // USBDEVFS_RESET, _IO('U', 20) in linux/usbdevice_fs.h.
    private const nuint UsbDevFsReset = 0x5514;

    /// <summary>A USB port reset, as the usbreset tool does it. The device keeps its driver and address.</summary>
    public static void Reset(string node)
    {
        try
        {
            using var handle = File.OpenHandle(node, FileMode.Open, FileAccess.Write, FileShare.ReadWrite);
            if (Ioctl(handle, UsbDevFsReset, 0) != 0)
            {
                throw new UsbHostException($"The kernel refused to reset {node} (errno {Marshal.GetLastPInvokeError()}).");
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            throw new UsbHostException($"Could not open {node} to reset it: {ex.Message}", ex);
        }
    }

    [LibraryImport("libc", EntryPoint = "ioctl", SetLastError = true)]
    private static partial int Ioctl(SafeFileHandle handle, nuint request, nint argument);
}
