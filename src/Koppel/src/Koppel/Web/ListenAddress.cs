using System.Net;

namespace Koppel.Web;

/// <summary>Where the web ports bind, from <c>Ports:Address</c>.</summary>
internal static class ListenAddress
{
    public const string Setting = "Ports:Address";

    /// <summary>
    /// The setting is an IP address. Unset, it is any address: other Koppel machines must reach the
    /// network port, and who may use the UI is <see cref="UiAccess"/>'s call. Null means any address.
    /// </summary>
    public static IPAddress? Resolve(string? setting)
    {
        if (string.IsNullOrWhiteSpace(setting))
        {
            return null;
        }

        return IPAddress.TryParse(setting.Trim(), out var address)
            ? address
            : throw new InvalidOperationException($"{Setting} is \"{setting}\", which is not an IP address. Use an address such as 127.0.0.1, ::1 or 0.0.0.0.");
    }
}
