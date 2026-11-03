using System.Net;
using System.Net.Sockets;
using System.Text.Json;

namespace Koppel.Core.Discovery;

/// <summary>
/// How a client finds Koppel servers on the local network. It broadcasts a probe over UDP to the port
/// number Koppel's HTTPS runs on (5240 unless changed), and every server that hears it answers the
/// client directly with what it needs to pair, its HTTPS port included. TCP and UDP ports are separate,
/// so a Koppel machine needs that one port number for everything.
/// </summary>
public static class KoppelService
{
    public const int DefaultPort = 5240;

    /// <summary>
    /// Probes are padded to this size, and a server never answers with more bytes than the probe had,
    /// so nobody can use a Koppel server to multiply traffic aimed at someone else.
    /// </summary>
    public const int ProbeSize = 512;

    internal static JsonSerializerOptions Json { get; } = new(JsonSerializerDefaults.Web);

    /// <summary>The probe's first line. The <c>/1</c> lets the format change later without confusing older servers.</summary>
    internal static ReadOnlySpan<byte> ProbeLine => "KOPPEL-DISCOVER/1\n"u8;

    /// <summary>The reply's first line. JSON with the server's details follows.</summary>
    internal static ReadOnlySpan<byte> ReplyLine => "KOPPEL-SERVER/1\n"u8;

    public static byte[] CreateProbe()
    {
        var probe = new byte[ProbeSize];
        ProbeLine.CopyTo(probe);
        return probe;
    }

    public static bool IsProbe(ReadOnlySpan<byte> datagram) => datagram.Length >= ProbeSize && datagram.StartsWith(ProbeLine);

    /// <summary>The answer to a probe. The name is cut to what a client keeps, so the answer always fits in a probe's size.</summary>
    public static byte[] CreateReply(ServerAnnouncement announcement)
    {
        ArgumentNullException.ThrowIfNull(announcement);
        var body = JsonSerializer.SerializeToUtf8Bytes(
            new ReplyBody(
                ServerBrowser.Bound(announcement.Name, ServerBrowser.MaxNameLength),
                announcement.HttpsPort,
                announcement.Fingerprint,
                ServerBrowser.Bound(announcement.Version, ServerBrowser.MaxVersionLength)),
            Json);
        return [.. ReplyLine, .. body];
    }

    internal sealed record ReplyBody(string? Name, int Port, string? Fingerprint, string? Version);
}

/// <summary>What a server tells clients that look for it.</summary>
/// <param name="Name">The machine name people know it by.</param>
/// <param name="HttpsPort">Where its token endpoint and pairing endpoints listen.</param>
/// <param name="Fingerprint">SHA-256 of its certificate, which a client pins when it pairs.</param>
/// <param name="Version">Koppel's version, for the UI.</param>
public sealed record ServerAnnouncement(string Name, int HttpsPort, string Fingerprint, string Version);

/// <summary>A server found on the network.</summary>
/// <param name="Address">The address its answer came from.</param>
public sealed record DiscoveredServer(string Name, string Address, int HttpsPort, string Fingerprint, string? Version)
{
    public string Url => IPAddress.TryParse(Address, out var ip) && ip.AddressFamily == AddressFamily.InterNetworkV6
        ? $"https://[{Address}]:{HttpsPort}"
        : $"https://{Address}:{HttpsPort}";
}
