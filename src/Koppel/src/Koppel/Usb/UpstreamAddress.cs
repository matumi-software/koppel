using System.Globalization;
using System.Net;

namespace Koppel.Usb;

/// <summary>The USB/IP server behind <c>Usb:Upstream</c>.</summary>
internal readonly record struct UpstreamAddress(string Host, int Port)
{
    public const string Setting = "Usb:Upstream";

    /// <summary>USB/IP's usual port.</summary>
    public const int DefaultPort = 3240;

    /// <summary>
    /// Accepts <c>host</c>, <c>host:port</c>, <c>[v6]:port</c>, <c>[v6]</c> and a bare IPv6 address; the
    /// port defaults to 3240. A value that is none of those throws with a message naming the setting.
    /// </summary>
    public static UpstreamAddress Parse(string value)
    {
        ArgumentNullException.ThrowIfNull(value);
        var text = value.Trim();
        if (text.StartsWith('['))
        {
            var close = text.IndexOf(']', StringComparison.Ordinal);
            if (close < 0 || !IPAddress.TryParse(text[1..close], out var v6) || v6.AddressFamily != System.Net.Sockets.AddressFamily.InterNetworkV6)
            {
                throw Bad(value, "the text in [ ] is not an IPv6 address");
            }

            var rest = text[(close + 1)..];
            return rest.Length == 0 ? new(text[1..close], DefaultPort) : new(text[1..close], PortAfterColon(value, rest));
        }

        var colons = text.Count(c => c == ':');
        if (colons > 1)
        {
            // Several colons without brackets can only be a bare IPv6 address, with no room for a port.
            return IPAddress.TryParse(text, out var bare) && bare.AddressFamily == System.Net.Sockets.AddressFamily.InterNetworkV6
                ? new(text, DefaultPort)
                : throw Bad(value, "put an IPv6 address in [ ] when it has a port");
        }

        var host = colons == 1 ? text[..text.IndexOf(':', StringComparison.Ordinal)] : text;
        if (host.Length == 0 || host.Any(char.IsWhiteSpace))
        {
            throw Bad(value, "the host is missing");
        }

        return colons == 1 ? new(host, PortAfterColon(value, text[host.Length..])) : new(host, DefaultPort);
    }

    private static int PortAfterColon(string value, string colonAndPort) =>
        colonAndPort.StartsWith(':') && int.TryParse(colonAndPort[1..], NumberStyles.None, CultureInfo.InvariantCulture, out var port) && port is >= 1 and <= 65535
            ? port
            : throw Bad(value, "the port must be a number from 1 to 65535");

    private static InvalidOperationException Bad(string value, string why) =>
        new($"{Setting} is \"{value}\": {why}. Use host, host:port or [v6]:port.");
}
