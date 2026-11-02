using System.Text.RegularExpressions;

namespace Koppel.Core.Usb;

/// <summary>
/// A kernel bus ID such as <c>1-1.4</c>: a bus number, a dash, then the port path. Bus IDs from the
/// network and from a local USB/IP driver are checked with this before they are used, so one cannot
/// carry anything into a command line, a log line or a UI row.
/// </summary>
public static partial class UsbBusId
{
    /// <summary>The kernel's own limit (<c>SYSFS_BUS_ID_SIZE</c> is 32, with the terminator).</summary>
    public const int MaxLength = 31;

    public static bool IsValid(string? busId) =>
        busId is { Length: > 0 and <= MaxLength } && Pattern().IsMatch(busId);

    /// <summary>Returns the bus ID, or throws <see cref="UsbHostException"/> when it is not one.</summary>
    public static string Require(string? busId) =>
        IsValid(busId) ? busId! : throw new UsbHostException("That is not a USB bus ID.");

    [GeneratedRegex(@"\A[0-9]+-[0-9]+(\.[0-9]+)*\z", RegexOptions.CultureInvariant)]
    private static partial Regex Pattern();
}
