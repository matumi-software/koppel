using System.Net;

namespace Koppel.Web;

/// <summary>Where the UI's HTTP port binds, from <c>Ports:HttpAddress</c>.</summary>
internal static class ListenAddress
{
    public const string Setting = "Ports:HttpAddress";

    /// <summary>
    /// The setting is an IP address. Unset, it is any address (a Docker container on a headless thin
    /// client needs the LAN) outside Development, where it is loopback. The installer sets
    /// 127.0.0.1 for native Windows. Null means any address.
    /// </summary>
    public static IPAddress? Resolve(string? setting, bool development)
    {
        if (string.IsNullOrWhiteSpace(setting))
        {
            return development ? IPAddress.Loopback : null;
        }

        return IPAddress.TryParse(setting.Trim(), out var address)
            ? address
            : throw new InvalidOperationException($"{Setting} is \"{setting}\", which is not an IP address. Use an address such as 127.0.0.1, ::1 or 0.0.0.0.");
    }
}
