using System.Net;
using System.Net.Sockets;

namespace Koppel.Core.Discovery;

/// <summary>
/// How a Koppel server shows itself on the local network: a DNS-SD service of type
/// <c>_koppel._tcp</c> on its HTTPS port, with TXT records for what a client needs to pair.
/// </summary>
public static class KoppelService
{
    public const string Type = "_koppel._tcp";

    /// <summary>TXT keys. <c>v</c> lets the format change later without confusing older clients.</summary>
    public const string ProtocolKey = "v";
    public const string Protocol = "1";
    public const string NameKey = "name";
    public const string FingerprintKey = "fp";
    public const string VersionKey = "version";
}

/// <summary>What a server announces.</summary>
/// <param name="Name">The machine name people know it by.</param>
/// <param name="HttpsPort">Where its token endpoint and pairing endpoints listen.</param>
/// <param name="Fingerprint">SHA-256 of its certificate, which a client pins when it pairs.</param>
/// <param name="Version">Koppel's version, for the UI.</param>
public sealed record ServerAnnouncement(string Name, int HttpsPort, string Fingerprint, string Version);

/// <summary>A server found on the network.</summary>
/// <param name="Address">The address to reach it on, picked from those it announced.</param>
public sealed record DiscoveredServer(string Name, string Address, int HttpsPort, string Fingerprint, string? Version)
{
    public string Url => IPAddress.TryParse(Address, out var ip) && ip.AddressFamily == AddressFamily.InterNetworkV6
        ? $"https://[{Address}]:{HttpsPort}"
        : $"https://{Address}:{HttpsPort}";
}
